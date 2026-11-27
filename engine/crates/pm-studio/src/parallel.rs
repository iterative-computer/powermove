//! Bounded CPU work on independent image rows. Pixel arithmetic stays unchanged.
pub(crate) fn rows(out: &mut [f32], stride: usize, work: impl Fn(usize, &mut [f32]) + Sync) {
    let workers = if out.len() >= 300_000 {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
    } else {
        1
    };
    rows_with_workers(out, stride, workers, work);
}
fn rows_with_workers(
    out: &mut [f32],
    stride: usize,
    workers: usize,
    work: impl Fn(usize, &mut [f32]) + Sync,
) {
    assert!(stride > 0 && out.len().is_multiple_of(stride));
    if workers == 1 || out.is_empty() {
        for (y, row) in out.chunks_mut(stride).enumerate() {
            work(y, row);
        }
        return;
    }
    let rows_per = (out.len() / stride).div_ceil(workers);
    std::thread::scope(|scope| {
        for (group, chunk) in out.chunks_mut(rows_per * stride).enumerate() {
            let work = &work;
            scope.spawn(move || {
                for (y, row) in chunk.chunks_mut(stride).enumerate() {
                    work(group * rows_per + y, row);
                }
            });
        }
    });
}
#[cfg(test)]
mod tests {
    #[test]
    fn uneven_partitions_visit_each_row_once_without_changing_values() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for (height, workers) in [(1, 8), (31, 7), (1033, 8)] {
            let hits: Vec<_> = (0..height).map(|_| AtomicUsize::new(0)).collect();
            let mut out = vec![-1.; height * 11];
            super::rows_with_workers(&mut out, 11, workers, |y, row| {
                hits[y].fetch_add(1, Ordering::Relaxed);
                row.fill(y as f32);
            });
            assert!(hits.iter().all(|n| n.load(Ordering::Relaxed) == 1));
            assert!(
                out.chunks_exact(11)
                    .enumerate()
                    .all(|(y, r)| r.iter().all(|v| *v == y as f32))
            );
        }
    }
}
