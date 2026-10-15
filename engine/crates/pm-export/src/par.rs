//! Row-parallel loops for the CPU encoder input and readback conversions.
//!
//! Workers are scoped threads that take the calling thread's QoS class first: on
//! macOS a new thread starts at the default class, and the job engine runs at
//! utility so it never competes with the interactive engine for CPU (contract 13,
//! invariant 10).

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
    if threads <= 1 {
        for (y, row) in out.chunks_mut(row_len).enumerate() {
            f(y, row);
        }
        return;
    }
    let per = total.div_ceil(threads);
    let qos = current_qos();
    let f = &f;
    std::thread::scope(|scope| {
        for (band, chunk) in out.chunks_mut(per * row_len).enumerate() {
            scope.spawn(move || {
                apply_qos(qos);
                for (i, row) in chunk.chunks_mut(row_len).enumerate() {
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
    (status == 0 && class as u32 != libc::qos_class_t::QOS_CLASS_UNSPECIFIED as u32).then_some(class)
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
}
