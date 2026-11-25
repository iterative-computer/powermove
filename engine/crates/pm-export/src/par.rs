//! Row-parallel loops for the CPU encoder input and readback conversions.
//!
//! Workers are scoped threads that take the calling thread's QoS class first.
//! A user export also refreshes its class while converting rows, so playback
//! transitions reach workers inside a frame (contract 13, invariant 10).

use std::any::Any;
use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex};

type Refresh = Arc<dyn Fn() + Send + Sync>;

struct WorkSlot {
    /// Raw pointer to `dyn Fn(usize) + Sync` living on the calling thread's
    /// stack. Workers run it only while `DispatchGuard` is alive on that stack.
    job: Option<(*const (dyn Fn(usize) + Sync), usize, Qos)>,
    generation: u64,
    pending: usize,
    panic: Option<Box<dyn Any + Send>>,
    shutdown: bool,
}

// SAFETY: the raw closure pointer refers to a `Sync` closure on the caller's
// stack and is cleared before `dispatch` returns (even on unwind).
unsafe impl Send for WorkSlot {}

struct WorkerPool {
    shared: Arc<(Mutex<WorkSlot>, Condvar, Condvar)>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl WorkerPool {
    fn new(worker_count: usize) -> Self {
        let shared = Arc::new((
            Mutex::new(WorkSlot {
                job: None,
                generation: 0,
                pending: 0,
                panic: None,
                shutdown: false,
            }),
            Condvar::new(),
            Condvar::new(),
        ));
        let mut workers = Vec::with_capacity(worker_count);
        for worker_idx in 0..worker_count {
            let band = worker_idx + 1;
            let shared = shared.clone();
            workers.push(std::thread::spawn(move || {
                let (lock, work_cv, done_cv) = &*shared;
                let mut seen = 0u64;
                loop {
                    let (job_ptr, active_bands, qos) = {
                        let mut slot = lock.lock().unwrap();
                        while slot.generation == seen && !slot.shutdown {
                            slot = work_cv.wait(slot).unwrap();
                        }
                        if slot.shutdown {
                            return;
                        }
                        seen = slot.generation;
                        match slot.job {
                            Some(job) => job,
                            None => continue,
                        }
                    };
                    if band < active_bands {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            apply_qos(qos);
                            // SAFETY: `dispatch` keeps the referent alive until
                            // every active worker decrements `pending` to 0.
                            let f = unsafe { &*job_ptr };
                            f(band);
                        }));
                        let mut slot = lock.lock().unwrap();
                        if let Err(err) = result
                            && slot.panic.is_none()
                        {
                            slot.panic = Some(err);
                        }
                        slot.pending -= 1;
                        if slot.pending == 0 {
                            done_cv.notify_one();
                        }
                    }
                }
            }));
        }
        Self { shared, workers }
    }

    fn dispatch<F: Fn(usize) + Sync>(&mut self, bands: usize, qos: Qos, f: F) {
        if bands <= 1 {
            f(0);
            return;
        }
        let needed_workers = bands - 1;
        if self.workers.len() < needed_workers {
            // Recreate with enough workers if a larger call arrives later.
            *self = Self::new(needed_workers);
        }
        let f_ref: &(dyn Fn(usize) + Sync) = &f;
        // Erase the stack lifetime; `DispatchGuard` guarantees all workers
        // finish before `f` is dropped, even if band 0 unwinds.
        let job_ptr: *const (dyn Fn(usize) + Sync) = unsafe {
            std::mem::transmute::<*const (dyn Fn(usize) + Sync), *const (dyn Fn(usize) + Sync)>(
                f_ref,
            )
        };
        let (lock, work_cv, _done_cv) = &*self.shared;
        {
            let mut slot = lock.lock().unwrap();
            slot.job = Some((job_ptr, bands, qos));
            slot.pending = needed_workers;
            slot.panic = None;
            slot.generation = slot.generation.wrapping_add(1);
            work_cv.notify_all();
        }
        struct DispatchGuard<'a> {
            shared: &'a (Mutex<WorkSlot>, Condvar, Condvar),
        }
        impl Drop for DispatchGuard<'_> {
            fn drop(&mut self) {
                let (lock, _, done_cv) = self.shared;
                let mut slot = lock.lock().unwrap();
                while slot.pending > 0 {
                    slot = done_cv.wait(slot).unwrap();
                }
                slot.job = None;
            }
        }
        let guard = DispatchGuard {
            shared: &self.shared,
        };
        let band0 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(0)));
        drop(guard);
        let worker_panic = lock.lock().unwrap().panic.take();
        if let Err(err) = band0 {
            std::panic::resume_unwind(err);
        }
        if let Some(err) = worker_panic {
            std::panic::resume_unwind(err);
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        let (lock, work_cv, _) = &*self.shared;
        {
            let mut slot = lock.lock().unwrap();
            slot.shutdown = true;
            work_cv.notify_all();
        }
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

thread_local! {
    static QOS_REFRESH: RefCell<Option<Refresh>> = RefCell::new(None);
    static WORKER_POOL: RefCell<Option<WorkerPool>> = RefCell::new(None);
}

fn dispatch_bands<F: Fn(usize) + Sync>(bands: usize, qos: Qos, f: F) {
    if bands <= 1 {
        f(0);
        return;
    }
    WORKER_POOL.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            let pool = guard.get_or_insert_with(|| WorkerPool::new(bands - 1));
            pool.dispatch(bands, qos, f);
        } else {
            // Re-entrant call from inside a band: run inline to avoid RefCell panic.
            for band in 0..bands {
                f(band);
            }
        }
    });
}

/// Installs a playback-aware refresh callback for conversions on this thread.
/// A nested call restores its predecessor when the scope exits.
#[cfg(all(target_os = "macos", any(feature = "engine", test)))]
pub(crate) fn install_qos_refresh(refresh: Option<Refresh>) -> RefreshGuard {
    let previous = QOS_REFRESH.with(|slot| slot.replace(refresh));
    RefreshGuard(previous)
}

#[cfg(all(target_os = "macos", any(feature = "engine", test)))]
pub(crate) struct RefreshGuard(Option<Refresh>);

#[cfg(all(target_os = "macos", any(feature = "engine", test)))]
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        QOS_REFRESH.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

#[cfg(all(feature = "engine", target_os = "macos"))]
pub(crate) fn refresh_current_qos() {
    let refresh = QOS_REFRESH.with(|slot| slot.borrow().clone());
    if let Some(refresh) = refresh {
        refresh();
    }
}

fn refresh_rows(refresh: Option<&Refresh>, row: usize) {
    if row.is_multiple_of(8)
        && let Some(refresh) = refresh
    {
        refresh();
    }
}

/// Most worker threads per call.
pub const MAX_THREADS: usize = 8;
/// Rows below which a call stays on the calling thread.
const MIN_ROWS_PER_THREAD: usize = 16;

/// Worker threads for `rows` rows.
fn threads_for(rows: usize, max_threads: usize) -> usize {
    let available = std::thread::available_parallelism().map_or(1, |n| n.get());
    available
        .min(max_threads)
        .min(rows / MIN_ROWS_PER_THREAD)
        .max(1)
}

/// Calls `f(row_index, row)` for every row of `out` (rows of `row_len` items),
/// splitting contiguous runs of rows across threads.
pub fn rows<T: Send>(out: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    rows_limited(out, row_len, MAX_THREADS, f);
}

/// Like [`rows`], with a call-specific worker cap. Preview rows use four
/// workers without changing the encoder and readback conversion limits.
pub(crate) fn rows_limited<T: Send>(
    out: &mut [T],
    row_len: usize,
    max_threads: usize,
    f: impl Fn(usize, &mut [T]) + Sync,
) {
    if row_len == 0 || out.is_empty() {
        return;
    }
    let total = out.len() / row_len;
    let threads = threads_for(total, max_threads);
    let refresh = QOS_REFRESH.with(|slot| slot.borrow().clone());
    if threads <= 1 {
        for (y, row) in out.chunks_mut(row_len).enumerate() {
            refresh_rows(refresh.as_ref(), y);
            f(y, row);
        }
        return;
    }
    let per = total.div_ceil(threads);
    let qos = current_qos();
    let f = &f;
    let out_ptr = out.as_mut_ptr() as usize;
    let out_len = out.len();
    let bands = out_len.div_ceil(per * row_len);
    dispatch_bands(bands, qos, |band| {
        let start = band * per * row_len;
        let end = (start + per * row_len).min(out_len);
        // SAFETY: bands partition `out` into strictly disjoint mutable ranges.
        let chunk =
            unsafe { std::slice::from_raw_parts_mut((out_ptr as *mut T).add(start), end - start) };
        for (i, row) in chunk.chunks_mut(row_len).enumerate() {
            refresh_rows(refresh.as_ref(), i);
            f(band * per + i, row);
        }
    });
}

/// Like [`rows`], with a state each worker makes once (`init`) and keeps
/// across its rows: scratch that would otherwise be allocated per row.
pub(crate) fn rows_with<T: Send, S>(
    out: &mut [T],
    row_len: usize,
    init: impl Fn() -> S + Sync,
    f: impl Fn(&mut S, usize, &mut [T]) + Sync,
) {
    planes_with(
        (out, row_len),
        (&mut [] as &mut [u8], 0),
        (&mut [] as &mut [u8], 0),
        init,
        |state, y, row, _, _| f(state, y, row),
    );
}

/// Calls `f(state, row_index, a_row, b_row, c_row)` for every row of up to
/// three planes cut in step (each a slice and its items per row; a plane
/// with no items per row is absent and its rows are empty), splitting
/// contiguous runs of rows across threads. Each worker makes its state once
/// (`init`) and keeps it across its rows. The first plane sets the row count.
pub(crate) fn planes_with<A: Send, B: Send, C: Send, S>(
    a: (&mut [A], usize),
    b: (&mut [B], usize),
    c: (&mut [C], usize),
    init: impl Fn() -> S + Sync,
    f: impl Fn(&mut S, usize, &mut [A], &mut [B], &mut [C]) + Sync,
) {
    let ((a, a_len), (b, b_len), (c, c_len)) = (a, b, c);
    if a_len == 0 || a.is_empty() {
        return;
    }
    let total = a.len() / a_len;
    assert!(
        a.len() % a_len == 0,
        "planes_with: the first plane ends in a partial row"
    );
    for (len, items) in [(b_len, b.len()), (c_len, c.len())] {
        assert!(
            len == 0 || total.checked_mul(len).is_some_and(|n| items >= n),
            "planes_with: a plane is shorter than the first plane's rows"
        );
    }
    let threads = threads_for(total, MAX_THREADS);
    let per = total.div_ceil(threads);
    let refresh = QOS_REFRESH.with(|slot| slot.borrow().clone());
    let (init, f) = (&init, &f);
    let band = |first: usize, a: &mut [A], b: &mut [B], c: &mut [C], refresh: Option<&Refresh>| {
        let mut state = init();
        for (i, a_row) in a.chunks_mut(a_len).enumerate() {
            refresh_rows(refresh, i);
            f(
                &mut state,
                first + i,
                a_row,
                &mut b[i * b_len..(i + 1) * b_len],
                &mut c[i * c_len..(i + 1) * c_len],
            );
        }
    };
    if threads <= 1 {
        band(0, a, b, c, refresh.as_ref());
        return;
    }
    // Every band's rows of every plane, split off with checked slicing
    // before any worker runs.
    let bands = total.div_ceil(per);
    let (mut a, mut b, mut c) = (a, b, c);
    let mut parts = Vec::with_capacity(bands);
    for n in 0..bands {
        let rows = per.min(total - n * per);
        let (a_here, a_rest) = std::mem::take(&mut a).split_at_mut(rows * a_len);
        let (b_here, b_rest) = std::mem::take(&mut b).split_at_mut(rows * b_len);
        let (c_here, c_rest) = std::mem::take(&mut c).split_at_mut(rows * c_len);
        (a, b, c) = (a_rest, b_rest, c_rest);
        parts.push(std::sync::Mutex::new(Some((
            n * per,
            a_here,
            b_here,
            c_here,
        ))));
    }
    let (qos, band, parts) = (current_qos(), &band, &parts);
    dispatch_bands(bands, qos, |n| {
        let (first, a, b, c) = parts[n]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("each band runs once");
        band(first, a, b, c, refresh.as_ref());
    });
}

#[cfg(target_vendor = "apple")]
pub(crate) type Qos = Option<libc::qos_class_t>;
#[cfg(not(target_vendor = "apple"))]
#[derive(Clone, Copy)]
pub(crate) struct Qos;

#[cfg(target_vendor = "apple")]
pub(crate) fn current_qos() -> Qos {
    let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
    let mut priority = 0;
    // SAFETY: both out-pointers are valid for writes.
    let status =
        unsafe { libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut priority) };
    (status == 0 && class as u32 != libc::qos_class_t::QOS_CLASS_UNSPECIFIED as u32)
        .then_some(class)
}

#[cfg(not(target_vendor = "apple"))]
pub(crate) fn current_qos() -> Qos {
    Qos
}

#[cfg(target_vendor = "apple")]
pub(crate) fn apply_qos(qos: Qos) {
    if let Some(class) = qos {
        // SAFETY: sets the calling thread's class; no pointers involved.
        unsafe {
            libc::pthread_set_qos_class_self_np(class, 0);
        }
    }
}

#[cfg(not(target_vendor = "apple"))]
pub(crate) fn apply_qos(_: Qos) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_vendor = "apple")]
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A secondary plane shorter than the first plane's rows is refused
    /// before any band is cut (the parallel path once cut it unchecked).
    #[test]
    #[should_panic(expected = "shorter than the first plane's rows")]
    fn a_short_secondary_plane_is_refused() {
        let mut a = vec![0u8; 4096];
        let mut b: Vec<u8> = Vec::new();
        planes_with(
            (&mut a, 1),
            (&mut b, 1),
            (&mut [] as &mut [u8], 0),
            || (),
            |_, _, _, b_row, _| assert_eq!(b_row.len(), 1),
        );
    }

    #[test]
    #[should_panic(expected = "partial row")]
    fn a_partial_first_row_is_refused() {
        let mut a = vec![0u8; 4097];
        planes_with(
            (&mut a, 2),
            (&mut [] as &mut [u8], 0),
            (&mut [] as &mut [u8], 0),
            || (),
            |_, _, _, _, _| {},
        );
    }

    #[test]
    fn every_row_is_visited_once_with_its_index() {
        for rows_n in [1usize, 15, 16, 17, 100, 1081] {
            let mut v = vec![0usize; rows_n * 3];
            rows(&mut v, 3, |y, row| {
                for o in row.iter_mut() {
                    *o += y + 1;
                }
            });
            for (y, row) in v.chunks(3).enumerate() {
                assert!(row.iter().all(|&o| o == y + 1), "row {y} of {rows_n}");
            }
        }
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn workers_take_the_callers_qos_class() {
        std::thread::spawn(|| {
            unsafe {
                libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
            }
            let mut v = vec![None; 64 * 4];
            rows(&mut v, 4, |_, row| {
                let class = current_qos();
                for o in row.iter_mut() {
                    *o = class;
                }
            });
            assert!(v.iter().all(|c| {
                c.map(|c| c as u32) == Some(libc::qos_class_t::QOS_CLASS_UTILITY as u32)
            }));
        })
        .join()
        .unwrap();
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn a_row_worker_changes_qos_during_conversion() {
        let playing = Arc::new(AtomicBool::new(false));
        let switched = playing.clone();
        let held = playing.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let refresh: Refresh = Arc::new(move || {
                let class = if switched.load(Ordering::Acquire) {
                    libc::qos_class_t::QOS_CLASS_UTILITY
                } else {
                    libc::qos_class_t::QOS_CLASS_DEFAULT
                };
                apply_qos(Some(class));
            });
            let _refresh = install_qos_refresh(Some(refresh));
            let mut classes = vec![""; 256];
            rows(&mut classes, 1, |y, row| {
                row[0] = match current_qos() {
                    Some(libc::qos_class_t::QOS_CLASS_DEFAULT) => "default",
                    Some(libc::qos_class_t::QOS_CLASS_UTILITY) => "utility",
                    _ => "other",
                };
                if y == 0 {
                    ready_tx.send(()).unwrap();
                    while !held.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                }
            });
            classes
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        // Release the first band after playback begins. It still has rows to
        // convert, and must lower its own class without waiting for a frame.
        playing.store(true, Ordering::Release);
        let classes = worker.join().unwrap();
        assert_eq!(classes[0], "default");
        assert_eq!(classes[16], "utility");
    }
}
