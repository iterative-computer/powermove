//! Exhaustive frame math to 10^8 frames at every supported rate, both signs
//! (section 2 Time bar: "Frame math verified headlessly to 10⁸ frames at every
//! supported rate, including negative frames").
//!
//! Ignored by default because it performs 3.2 × 10⁹ checks; run it optimized:
//!
//! ```sh
//! cargo test --release -p pm-time --test exhaustive -- --ignored --nocapture
//! ```

use pm_time::{Rate, SUPPORTED_RATES};

const LIMIT: i64 = 100_000_000;

fn check_rate(rate: Rate, from: i64, to: i64) -> u64 {
    let ticks = rate
        .exact_frame_ticks()
        .expect("supported rates divide the flick");
    let mut checked = 0u64;
    let mut previous = rate.frame_start(from - 1).unwrap();
    for n in from..=to {
        let start = rate.frame_start(n).unwrap();
        assert_eq!(start, n * ticks, "{rate} frame {n}");
        assert!(start > previous, "{rate} frame {n} not increasing");
        assert_eq!(rate.frame_of(start), n, "{rate} frame {n}");
        assert_eq!(rate.frame_of(start - 1), n - 1, "{rate} frame {n} boundary");
        previous = start;
        checked += 1;
    }
    checked
}

/// Worker threads for the exhaustive run: `PM_EXHAUSTIVE_THREADS`, default 4
/// (the shared machine's limit).
fn workers() -> usize {
    std::env::var("PM_EXHAUSTIVE_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or(4)
}

#[test]
#[ignore = "3.2e9 checks; run with --release -- --ignored"]
fn frame_math_exhaustive_to_1e8_every_supported_rate() {
    let started = std::time::Instant::now();
    let jobs: Vec<(Rate, i64, i64)> = SUPPORTED_RATES
        .iter()
        .flat_map(|&rate| [(rate, -LIMIT, -1), (rate, 0, LIMIT)])
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let total: u64 = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers())
            .map(|_| {
                scope.spawn(|| {
                    let mut checked = 0;
                    loop {
                        let job = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&(rate, from, to)) = jobs.get(job) else {
                            return checked;
                        };
                        checked += check_rate(rate, from, to);
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(total, SUPPORTED_RATES.len() as u64 * (2 * LIMIT as u64 + 1));
    println!(
        "checked {total} frames across {} rates in {:.1?} on {} threads",
        SUPPORTED_RATES.len(),
        started.elapsed(),
        workers()
    );
}

#[test]
fn frame_math_exhaustive_to_1e6_every_supported_rate() {
    for rate in SUPPORTED_RATES {
        check_rate(rate, -1_000_000, 1_000_000);
    }
}
