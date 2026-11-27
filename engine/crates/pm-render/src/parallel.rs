//! Work a frame splits over threads: the misses of a cache it met new (a
//! text layer's glyph geometry, a shape layer's outlines), on the document
//! passes' threads (`pm_model::parallel`). Results never depend on the
//! thread count.

/// `f(0), …, f(n - 1)` in order, on this thread and up to
/// `pm_model::parallel::workers() - 1` helpers that persist across frames
/// and run at the calling thread's quality of service. macOS starts a new
/// thread at the default class, and a render thread at user-interactive
/// would wait for helpers the scheduler holds back (on efficiency cores,
/// behind other work) for the frame it is drawing. Starting a thread per
/// helper per call cost more than the work itself (thread start, a fresh
/// allocator heap whose pages are committed and purged each frame): a frame
/// with new text or shape geometry calls this once per layer.
pub(crate) fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        pool::map(n, qos::current(), &f)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (0..n).map(f).collect()
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod pool {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
    use std::sync::{Arc, Condvar, Mutex, OnceLock};

    /// One call's work, shared with helpers by address. A helper runs it
    /// only while the call is open, and the caller closes it and waits for
    /// the helpers that started it before the borrow ends.
    struct Task {
        work: *const (dyn Fn() + Sync),
        qos: u32,
        call: Arc<Call>,
    }
    // SAFETY: `work` points at a `Sync` closure that outlives every use: a
    // helper reads it only after counting itself in `running` while the call
    // is open, and the caller closes the call and waits for `running` to
    // reach zero before returning.
    unsafe impl Send for Task {}

    #[derive(Default)]
    struct Call {
        state: Mutex<CallState>,
        done: Condvar,
    }

    #[derive(Default)]
    struct CallState {
        /// Helpers inside `work`.
        running: usize,
        /// The caller finished: a helper that dequeues the task now skips
        /// it. A helper busy with another caller's work (another renderer)
        /// is never waited for.
        closed: bool,
        panic: Option<Box<dyn std::any::Any + Send>>,
    }

    /// Tests: tasks sent to helpers and not yet taken by them.
    #[cfg(test)]
    pub(super) static QUEUED: AtomicUsize = AtomicUsize::new(0);

    /// A helper thread: a one-task mailbox, and whether it is free. A call
    /// claims a free helper before handing it a task, so a task never waits
    /// behind another caller's work; with no helper free the caller does
    /// all of its work itself.
    struct Helper {
        mailbox: SyncSender<Task>,
        free: Arc<AtomicBool>,
    }

    impl Helper {
        /// Hands `task` over if this helper is free.
        fn offer(&self, task: impl FnOnce() -> Task) -> bool {
            if self
                .free
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return false;
            }
            // A helper whose thread is gone stays claimed: never asked again.
            match self.mailbox.try_send(task()) {
                Ok(()) => {
                    #[cfg(test)]
                    QUEUED.fetch_add(1, Ordering::AcqRel);
                    true
                }
                Err(_) => false,
            }
        }
    }

    struct Pool {
        helpers: Vec<Helper>,
    }

    /// A call's state, also when a panic poisoned its lock: the panic was
    /// caught and is carried in `CallState::panic`.
    fn state(call: &Call) -> std::sync::MutexGuard<'_, CallState> {
        call.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pool() -> &'static Pool {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(|| {
            let count = pm_model::parallel::workers().saturating_sub(1);
            let helpers = (0..count)
                .filter_map(|i| {
                    let (mailbox, tasks): (SyncSender<Task>, Receiver<Task>) = sync_channel(1);
                    let free = Arc::new(AtomicBool::new(true));
                    let freed = free.clone();
                    std::thread::Builder::new()
                        .name(format!("pm-render-helper-{i}"))
                        .spawn(move || {
                            for task in tasks {
                                #[cfg(test)]
                                QUEUED.fetch_sub(1, Ordering::AcqRel);
                                run(task);
                                freed.store(true, Ordering::Release);
                            }
                        })
                        .ok()
                        .map(|_| Helper { mailbox, free })
                })
                .collect();
            Pool { helpers }
        })
    }

    /// A helper's turn at a call: skipped once the call is closed.
    fn run(task: Task) {
        {
            let mut state = state(&task.call);
            if state.closed {
                return;
            }
            state.running += 1;
        }
        super::qos::adopt(task.qos);
        // SAFETY: see `Task`.
        let work = unsafe { &*task.work };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        let mut state = state(&task.call);
        state.running -= 1;
        if let Err(panic) = result {
            state.panic.get_or_insert(panic);
        }
        task.call.done.notify_all();
    }

    /// Tests: sends a free helper a closed call whose lock a panic
    /// poisoned, then offers every helper a call that reports it ran: each
    /// must run it. `None`: a helper was busy with another test's work.
    #[cfg(test)]
    pub(super) fn poisoned_call_then_probe() -> Option<bool> {
        let poisoned = Arc::new(Call::default());
        let held = poisoned.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.state.lock().unwrap();
            panic!("a task panicked with the call's lock held");
        })
        .join();
        assert!(poisoned.state.is_poisoned());
        state(&poisoned).closed = true;
        static NOOP: fn() = || {};
        let noop: &'static (dyn Fn() + Sync) = &NOOP;
        let mut task = Some(Task {
            work: noop,
            qos: 0,
            call: poisoned,
        });
        if !pool()
            .helpers
            .iter()
            .any(|h| h.offer(|| task.take().unwrap()))
        {
            return None;
        }
        let (ran, probe) = std::sync::mpsc::channel();
        let ran = Mutex::new(ran);
        let report = move || {
            let _ = ran.lock().unwrap().send(());
        };
        let report: &'static (dyn Fn() + Sync) = Box::leak(Box::new(report));
        let call = Arc::new(Call::default());
        // The poisoned call's helper frees itself unless its thread died.
        let started = std::time::Instant::now();
        while !pool()
            .helpers
            .iter()
            .all(|h| h.free.load(Ordering::Acquire))
        {
            if started.elapsed() > std::time::Duration::from_secs(5) {
                return Some(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let sent = pool()
            .helpers
            .iter()
            .filter(|helper| {
                helper.offer(|| Task {
                    work: report,
                    qos: 0,
                    call: call.clone(),
                })
            })
            .count();
        let ran = (0..sent)
            .filter(|_| {
                probe
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .is_ok()
            })
            .count();
        // Another test took a helper meanwhile: try again.
        (sent == pool().helpers.len()).then_some(ran == sent)
    }

    pub(super) fn map<T: Send>(n: usize, qos: u32, f: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
        let next = AtomicUsize::new(0);
        let results: Mutex<Vec<(usize, T)>> = Mutex::new(Vec::with_capacity(n));
        let work = || {
            let mut done = Vec::new();
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                done.push((i, f(i)));
            }
            results.lock().unwrap().extend(done);
        };
        let call = Arc::new(Call::default());
        let work_ref: &(dyn Fn() + Sync) = &work;
        // SAFETY: erases the borrow's lifetime; `wait` below outlives every use.
        let ptr: *const (dyn Fn() + Sync) = unsafe { std::mem::transmute(work_ref) };
        // Free helpers only: one busy with another caller's work (another
        // renderer) is never queued behind; with none free the caller does
        // all of it.
        let mut wanted = n.saturating_sub(1);
        for helper in &pool().helpers {
            if wanted == 0 {
                break;
            }
            if helper.offer(|| Task {
                work: ptr,
                qos,
                call: call.clone(),
            }) {
                wanted -= 1;
            }
        }
        // The caller's own share must not unwind while helpers still read
        // `work`: close the call, wait for the helpers inside it, then
        // rethrow. Every index is taken once the caller's share returns.
        let mine = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&work));
        let mut state = state(&call);
        state.closed = true;
        while state.running > 0 {
            state = call.done.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        let theirs = state.panic.take();
        // Never unwind holding the call's lock: a helper taking the closed
        // call later must find it unpoisoned.
        drop(state);
        if let Err(panic) = mine {
            std::panic::resume_unwind(panic);
        }
        if let Some(panic) = theirs {
            std::panic::resume_unwind(panic);
        }
        let mut pairs = results.into_inner().unwrap();
        pairs.sort_unstable_by_key(|(i, _)| *i);
        pairs.into_iter().map(|(_, v)| v).collect()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "parallel_tests.rs"]
mod parallel_tests;

#[cfg(target_os = "macos")]
mod qos {
    use std::cell::Cell;

    unsafe extern "C" {
        fn qos_class_self() -> u32;
        fn pthread_set_qos_class_self_np(qos: u32, relative: i32) -> i32;
    }

    thread_local! { static ADOPTED: Cell<u32> = const { Cell::new(0) }; }

    pub(super) fn current() -> u32 {
        // SAFETY: reads the calling thread's class.
        unsafe { qos_class_self() }
    }

    /// Runs this thread at `qos` from now on (once per thread).
    pub(super) fn adopt(qos: u32) {
        ADOPTED.with(|adopted| {
            if adopted.get() != qos && current() != qos {
                // SAFETY: affects only the calling thread.
                unsafe { pthread_set_qos_class_self_np(qos, 0) };
            }
            adopted.set(qos);
        });
    }
}

#[cfg(not(target_os = "macos"))]
mod qos {
    pub(super) fn current() -> u32 {
        0
    }
    pub(super) fn adopt(_: u32) {}
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn helpers_run_at_the_callers_quality_of_service() {
        // A render thread runs at user-interactive; new threads start at
        // the default class.
        const USER_INTERACTIVE: u32 = 0x21;
        let classes = std::thread::spawn(|| {
            super::qos::adopt(USER_INTERACTIVE);
            super::map(64, |_| {
                std::thread::sleep(std::time::Duration::from_micros(200));
                super::qos::current()
            })
        })
        .join()
        .unwrap();
        assert_eq!(classes.len(), 64);
        assert!(
            classes.iter().all(|&c| c == USER_INTERACTIVE),
            "{classes:x?}"
        );
    }
}
