//! Host time in nanoseconds on the `mach_absolute_time` timebase.
//!
//! Every process in the spike uses this clock, so timestamps from A and B (and the
//! Metal host-time domain, which is also `mach_absolute_time`) are directly comparable.

use std::sync::OnceLock;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
    fn mach_wait_until(deadline: u64) -> i32;
}

fn timebase() -> (u64, u64) {
    static TB: OnceLock<(u64, u64)> = OnceLock::new();
    *TB.get_or_init(|| {
        let mut info = MachTimebaseInfo::default();
        // SAFETY: plain out-parameter call.
        let rc = unsafe { mach_timebase_info(&mut info) };
        assert_eq!(rc, 0, "mach_timebase_info failed");
        (info.numer as u64, info.denom as u64)
    })
}

/// Raw ticks.
pub fn ticks() -> u64 {
    // SAFETY: no arguments, always safe.
    unsafe { mach_absolute_time() }
}

pub fn ticks_to_ns(t: u64) -> u64 {
    let (n, d) = timebase();
    ((t as u128 * n as u128) / d as u128) as u64
}

pub fn ns_to_ticks(ns: u64) -> u64 {
    let (n, d) = timebase();
    ((ns as u128 * d as u128) / n as u128) as u64
}

/// Host time in nanoseconds.
pub fn now_ns() -> u64 {
    ticks_to_ns(ticks())
}

/// Block the calling thread until host time `deadline_ns` (absolute).
pub fn sleep_until_ns(deadline_ns: u64) {
    if deadline_ns <= now_ns() {
        return;
    }
    // SAFETY: plain call; returns early only on interruption, which we retry.
    loop {
        unsafe { mach_wait_until(ns_to_ticks(deadline_ns)) };
        if now_ns() >= deadline_ns {
            return;
        }
    }
}

pub fn sleep_ns(ns: u64) {
    sleep_until_ns(now_ns() + ns);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip() {
        for ns in [0u64, 1_000, 16_666_667, 60_000_000_000] {
            let back = ticks_to_ns(ns_to_ticks(ns));
            assert!(back.abs_diff(ns) <= 42, "{ns} -> {back}");
        }
    }

    #[test]
    fn sleep_until_is_not_early() {
        let start = now_ns();
        let target = start + 2_000_000;
        sleep_until_ns(target);
        let woke = now_ns();
        assert!(woke >= target);
        assert!(woke - target < 50_000_000, "overslept by {} ns", woke - target);
    }
}
