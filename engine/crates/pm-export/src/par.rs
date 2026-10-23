//! Row-parallel loops for the CPU encoder input and readback conversions.
//!
//! Workers are scoped threads that take the calling thread's QoS class first.
//! A user export also refreshes its class while converting rows, so playback
//! transitions reach workers inside a frame (contract 13, invariant 10).

use std::cell::RefCell;
use std::sync::Arc;

type Refresh = Arc<dyn Fn() + Send + Sync>;

thread_local! {
    static QOS_REFRESH: RefCell<Option<Refresh>> = RefCell::new(None);
}

/// Installs a playback-aware refresh callback for conversions on this thread.
/// A nested call restores its predecessor when the scope exits.
pub(crate) fn install_qos_refresh(refresh: Option<Refresh>) -> RefreshGuard {
    let previous = QOS_REFRESH.with(|slot| slot.replace(refresh));
    RefreshGuard(previous)
}

pub(crate) struct RefreshGuard(Option<Refresh>);

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        QOS_REFRESH.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

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
fn threads_for(rows: usize) -> usize {
    let available = std::thread::available_parallelism().map_or(1, |n| n.get());
    available
        .min(MAX_THREADS)
        .min(rows / MIN_ROWS_PER_THREAD)
        .max(1)
}

/// Calls `f(row_index, row)` for every row of `out` (rows of `row_len` items),
/// splitting contiguous runs of rows across threads.
pub fn rows<T: Send>(out: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if row_len == 0 || out.is_empty() {
        return;
    }
    let total = out.len() / row_len;
    let threads = threads_for(total);
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
    std::thread::scope(|scope| {
        for (band, chunk) in out.chunks_mut(per * row_len).enumerate() {
            let refresh = refresh.clone();
            scope.spawn(move || {
                apply_qos(qos);
                for (i, row) in chunk.chunks_mut(row_len).enumerate() {
                    refresh_rows(refresh.as_ref(), i);
                    f(band * per + i, row);
                }
            });
        }
    });
}

#[cfg(target_vendor = "apple")]
pub(crate) type Qos = Option<libc::qos_class_t>;
#[cfg(not(target_vendor = "apple"))]
pub(crate) type Qos = ();

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
pub(crate) fn current_qos() -> Qos {}

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
