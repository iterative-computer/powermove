//! Spike S1 frame-bridge primitives shared by the engine addon (utility process) and
//! the bridge addon (Electron main): the Mach bootstrap rendezvous, audit-token and
//! code-signature verification, and the per-frame wire format.
#![cfg(target_os = "macos")]

pub mod audit;
pub mod ffi;
pub mod rendezvous;
pub mod wire;

/// Microseconds on `CLOCK_UPTIME_RAW` (mach_absolute_time), the clock behind
/// Chromium's `TimeTicks` on macOS. Every timestamp in the spike uses it.
pub fn now_us() -> i64 {
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock: libc::clockid_t) -> u64;
    }
    // SAFETY: clock_gettime_nsec_np has no preconditions.
    (unsafe { clock_gettime_nsec_np(libc::CLOCK_UPTIME_RAW) } / 1000) as i64
}
