//! Process-wide decode counters, read by the headless benchmark. Relaxed
//! atomics only: they never steer decoding.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

macro_rules! counters {
    ($($name:ident = $label:literal,)*) => {
        $(pub static $name: AtomicU64 = AtomicU64::new(0);)*
        /// Every counter by name, as now.
        pub fn snapshot() -> Vec<(&'static str, u64)> {
            vec![$(($label, $name.load(Relaxed)),)*]
        }
    };
}

counters! {
    SESSIONS_OPENED = "sessions_opened",
    SESSIONS_CLOSED_OVER_SHARE = "sessions_closed_over_share",
    SESSIONS_CLOSED_IDLE_FULL = "sessions_closed_idle_full",
    SESSIONS_RELEASED = "sessions_released",
    SEEKS = "seeks",
    CONTINUED = "continued",
    WINDOW_HITS = "window_hits",
    SAMPLES_FED = "samples_fed",
    DECODE_WAIT_US = "decode_wait_us",
    FETCH_CACHED = "fetch_cached",
    FETCH_STAND_INS = "fetch_stand_ins",
    FETCH_WAITS = "fetch_waits",
    FETCH_WAIT_US = "fetch_wait_us",
    WORKER_DECODED = "worker_decoded",
    WORKER_AHEAD = "worker_ahead",
    WORKER_STOPPED = "worker_stopped",
    OPEN_COLD_FRAMES = "open_cold_frames",
    OPEN_COLD_JOBS = "open_cold_jobs",
    OUTLINE_HITS = "outline_hits",
    OUTLINE_MISSES = "outline_misses",
    OUTLINE_UNCACHEABLE = "outline_uncacheable",
    OUTLINE_PREPARED = "outline_prepared",
}

pub fn bump(counter: &AtomicU64, by: u64) {
    counter.fetch_add(by, Relaxed);
}

/// Benchmark switches (`PM_ABL_*` ablations, `PM_FIX_*` candidate fixes),
/// read once from the environment. Unset means today's behaviour.
pub fn knob(name: &str) -> Option<u64> {
    static KNOBS: std::sync::OnceLock<std::collections::HashMap<String, u64>> =
        std::sync::OnceLock::new();
    KNOBS
        .get_or_init(|| {
            std::env::vars()
                .filter(|(k, _)| k.starts_with("PM_ABL_") || k.starts_with("PM_FIX_") || k.starts_with("PM_SCRUB_"))
                .filter_map(|(k, v)| Some((k, v.parse().ok()?)))
                .collect()
        })
        .get(name)
        .copied()
}
