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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
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

    struct Pool {
        helpers: Mutex<Vec<Sender<Task>>>,
    }

    fn pool() -> &'static Pool {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(|| {
            let count = pm_model::parallel::workers().saturating_sub(1);
            let helpers = (0..count)
                .filter_map(|i| {
                    let (tx, rx): (Sender<Task>, Receiver<Task>) = channel();
                    std::thread::Builder::new()
                        .name(format!("pm-render-helper-{i}"))
                        .spawn(move || {
                            for task in rx {
                                {
                                    let mut state = task.call.state.lock().unwrap();
                                    if state.closed {
                                        continue;
                                    }
                                    state.running += 1;
                                }
                                super::qos::adopt(task.qos);
                                // SAFETY: see `Task`.
                                let work = unsafe { &*task.work };
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
                                let mut state = task.call.state.lock().unwrap();
                                state.running -= 1;
                                if let Err(panic) = result {
                                    state.panic.get_or_insert(panic);
                                }
                                task.call.done.notify_all();
                            }
                        })
                        .ok()
                        .map(|_| tx)
                })
                .collect();
            Pool {
                helpers: Mutex::new(helpers),
            }
        })
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
        {
            // Another frame's map (another renderer) runs its own work inline.
            if let Ok(helpers) = pool().helpers.try_lock() {
                let wanted = n.saturating_sub(1).min(helpers.len());
                for tx in helpers.iter().take(wanted) {
                    // A helper gone (its thread failed) is simply not asked.
                    let _ = tx.send(Task {
                        work: ptr,
                        qos,
                        call: call.clone(),
                    });
                }
            }
        }
        // The caller's own share must not unwind while helpers still read
        // `work`: close the call, wait for the helpers inside it, then
        // rethrow. Every index is taken once the caller's share returns.
        let mine = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&work));
        let mut state = call.state.lock().unwrap();
        state.closed = true;
        while state.running > 0 {
            state = call.done.wait(state).unwrap();
        }
        if let Err(panic) = mine {
            std::panic::resume_unwind(panic);
        }
        if let Some(panic) = state.panic.take() {
            std::panic::resume_unwind(panic);
        }
        drop(state);
        let mut pairs = results.into_inner().unwrap();
        pairs.sort_unstable_by_key(|(i, _)| *i);
        pairs.into_iter().map(|(_, v)| v).collect()
    }
}

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

#[cfg(test)]
mod pool_tests {
    /// Calls share one set of helper threads: none is started per call, and
    /// results come back in order whoever computed them.
    #[test]
    fn calls_reuse_the_same_helpers_and_keep_their_order() {
        let ids = || {
            super::map(256, |i| {
                std::thread::sleep(std::time::Duration::from_micros(50));
                (i, std::thread::current().id())
            })
        };
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            let out = ids();
            assert!(out.iter().enumerate().all(|(at, (i, _))| at == *i));
            seen.extend(out.into_iter().map(|(_, id)| id));
        }
        assert!(
            seen.len() <= pm_model::parallel::workers(),
            "{} threads for {} workers",
            seen.len(),
            pm_model::parallel::workers()
        );
    }

    /// Two renderers share the helpers. A second caller whose helpers are
    /// busy with the first caller's work does its own share and returns: it
    /// never waits for the other caller's work (it waited for all of it).
    #[test]
    fn a_second_caller_never_waits_for_the_first_callers_work() {
        use std::sync::{Condvar, Mutex};
        use std::time::{Duration, Instant};
        /// Holds every item of the first call until opened.
        #[derive(Default)]
        struct Gate {
            state: Mutex<(usize, bool)>,
            turn: Condvar,
        }
        impl Gate {
            fn enter(&self) {
                let mut g = self.state.lock().unwrap();
                g.0 += 1;
                self.turn.notify_all();
                while !g.1 {
                    g = self.turn.wait(g).unwrap();
                }
            }
            fn started(&self, n: usize) -> bool {
                let g = self.state.lock().unwrap();
                let (g, _) = self
                    .turn
                    .wait_timeout_while(g, Duration::from_secs(2), |g| g.0 < n)
                    .unwrap();
                g.0 >= n
            }
            fn open(&self) {
                self.state.lock().unwrap().1 = true;
                self.turn.notify_all();
            }
        }
        /// Opens the gate however the attempt ends.
        struct Opens<'a>(&'a Gate);
        impl Drop for Opens<'_> {
            fn drop(&mut self) {
                self.0.open();
            }
        }
        let workers = pm_model::parallel::workers();
        if workers < 2 {
            return;
        }
        // Another test asking for the helpers at the same instant makes the
        // first call run alone: that attempt proves nothing, try again.
        for _ in 0..20 {
            let gate = Gate::default();
            let attempt = std::thread::scope(|s| {
                let opens = Opens(&gate);
                let first = s.spawn(|| {
                    super::map(4 * workers, |i| {
                        gate.enter();
                        i
                    })
                });
                // The first caller and at least one helper are held.
                if !gate.started(2) {
                    return None;
                }
                let second = s.spawn(|| super::map(8, |i| i * 2));
                let deadline = Instant::now() + Duration::from_secs(5);
                while !second.is_finished() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(1));
                }
                let on_its_own = second.is_finished();
                drop(opens);
                assert_eq!(first.join().unwrap(), (0..4 * workers).collect::<Vec<_>>());
                Some((on_its_own, second.join().unwrap()))
            });
            if let Some((on_its_own, second)) = attempt {
                assert!(
                    on_its_own,
                    "the second call waited for the first call's work"
                );
                assert_eq!(second, (0..8).map(|i| i * 2).collect::<Vec<_>>());
                return;
            }
        }
        panic!("no helper ever started the first call");
    }
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
