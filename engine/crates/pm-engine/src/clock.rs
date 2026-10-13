//! The shared host clock (contract 6 and contract 13): a monotonic instant in
//! microseconds that every process on the machine reads the same way.
//!
//! On macOS this is `mach_absolute_time` (`CLOCK_UPTIME_RAW`), the domain of
//! Metal GPU timestamps on Apple silicon and of libuv's `uv_hrtime`, so
//! Electron main's `process.hrtime.bigint() / 1000n` reads the same instant
//! (checked by the main-process integration test).

use pm_protocol::common::HostMicros;

/// Now, in host microseconds.
pub fn host_now() -> HostMicros {
    HostMicros::new(raw_now_us()).unwrap_or(HostMicros::MAX)
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    // <time.h>, macOS 10.12+. Not bound by the libc crate.
    fn clock_gettime_nsec_np(clock_id: libc::clockid_t) -> u64;
}

#[cfg(target_vendor = "apple")]
fn raw_now_us() -> u64 {
    // SAFETY: clock_gettime_nsec_np has no preconditions; it returns 0 on error.
    let ns = unsafe { clock_gettime_nsec_np(libc::CLOCK_UPTIME_RAW) };
    ns / 1_000
}

#[cfg(all(unix, not(target_vendor = "apple")))]
fn raw_now_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec.
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } == 0;
    if !ok {
        return 0;
    }
    (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1_000
}

#[cfg(not(unix))]
fn raw_now_us() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// Adds microseconds to an instant, saturating at the top of the safe range.
pub fn after(instant: HostMicros, us: u64) -> HostMicros {
    HostMicros::new(instant.get().saturating_add(us)).unwrap_or(HostMicros::MAX)
}

/// Microseconds from `earlier` to `later`, or zero when `later` is earlier.
pub fn since(later: HostMicros, earlier: HostMicros) -> u64 {
    later.get().saturating_sub(earlier.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_clock_is_monotonic_and_nonzero() {
        let a = host_now();
        let b = host_now();
        assert!(a.get() > 0);
        assert!(b >= a);
        assert_eq!(since(a, b), 0);
        assert_eq!(after(HostMicros::MAX, 5), HostMicros::MAX);
    }
}
