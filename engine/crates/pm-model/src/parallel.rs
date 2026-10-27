//! Work on several threads for whole-document passes (parsing, the canonical
//! hash, the integrity digest). Results never depend on the thread count:
//! WebAssembly, which has no threads here, runs everything in turn.

/// The threads a whole-document pass may use (this one included).
pub fn workers() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static WORKERS: OnceLock<usize> = OnceLock::new();
        // Several engine processes share the machine: a pass takes at most 8.
        *WORKERS.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(8)))
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}

/// Runs `a` on another thread while `b` runs on this one.
pub fn join<A: Send, B>(a: impl FnOnce() -> A + Send, b: impl FnOnce() -> B) -> (A, B) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::scope(|scope| {
            let a = scope.spawn(a);
            let b = b();
            (
                a.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                b,
            )
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        (a(), b())
    }
}

/// `f(0), f(1), …, f(n - 1)` in order, computed on up to [`workers`] threads.
pub fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = workers().min(n);
    if threads < 2 {
        return (0..n).map(f).collect();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let next = AtomicUsize::new(0);
        let work = || {
            let mut done = Vec::new();
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    return done;
                }
                done.push((i, f(i)));
            }
        };
        let mut slots: Vec<Option<T>> = (0..n).map(|_| None).collect();
        std::thread::scope(|scope| {
            let helpers: Vec<_> = (1..threads).map(|_| scope.spawn(work)).collect();
            let mut place = |done: Vec<(usize, T)>| {
                for (i, value) in done {
                    slots[i] = Some(value);
                }
            };
            place(work());
            for helper in helpers {
                place(
                    helper
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                );
            }
        });
        slots
            .into_iter()
            .map(|slot| slot.expect("every index computed"))
            .collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        unreachable!("one worker")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_keeps_order_and_covers_every_index() {
        let out = map(1000, |i| i * 3);
        assert_eq!(out, (0..1000).map(|i| i * 3).collect::<Vec<_>>());
        assert!(map(0, |i| i).is_empty());
        assert_eq!(map(1, |i| i + 7), vec![7]);
    }

    #[test]
    fn join_runs_both() {
        assert_eq!(join(|| 1, || 2), (1, 2));
    }
}
