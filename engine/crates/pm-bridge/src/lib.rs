//! The frame bridge (plan section 18; amendments 4 and 5 in section 26).
//!
//! The interactive engine renders viewer frames into a pool of IOSurfaces.
//! This crate moves those surfaces from the engine's utility process to
//! Electron main, which imports each one once with
//! `sharedTexture.importSharedTexture` and sends it once to each viewer
//! that presents it. Per-frame descriptors never pass through here: they go
//! from the engine to the viewer over the viewer's `MessageChannelMain`
//! port.
//!
//! - [`rendezvous`]: the per-launch bootstrap name, one verified check-in,
//!   then an anonymous port checked against the engine's audit token.
//! - [`audit`]: audit tokens (pid and pidversion), parent and uid checks,
//!   and the code-signature check against main's Team ID.
//! - [`wire`]: the Mach message layouts; the parser is total and fuzzed
//!   because it runs in main.
//! - [`host`]: main's receive loop and the surfaces main holds.
//! - `node` (feature `node`): the addon Electron main loads.
//! - `fonts` (feature `node`): the font registry (section 12). Before
//!   Phase 2 the native font registry runs in main through this addon,
//!   answering `pm-wasm`'s registry queries and serving metrics-and-shaping
//!   subsets. From Phase 2 the engine owns it and main only forwards.
//!
//! Engines link this crate as an rlib (without `node`) for
//! [`rendezvous::Client`].

#[cfg(target_os = "macos")]
pub mod audit;
#[cfg(target_os = "macos")]
pub mod ffi;
#[cfg(target_os = "macos")]
pub mod host;
#[cfg(target_os = "macos")]
pub mod rendezvous;
#[cfg(target_os = "macos")]
pub mod wire;

// napi registers the exports; unit tests never call them.
#[cfg(all(target_os = "macos", feature = "node"))]
#[cfg_attr(test, allow(dead_code))]
mod node;

#[cfg(all(target_os = "macos", feature = "node"))]
mod fonts;

/// Microseconds on `CLOCK_UPTIME_RAW` (`mach_absolute_time`), the host
/// clock every engine and Chromium timestamp on macOS uses (contract 13).
#[cfg(target_os = "macos")]
pub fn now_us() -> i64 {
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock: libc::clockid_t) -> u64;
    }
    // SAFETY: clock_gettime_nsec_np has no preconditions.
    (unsafe { clock_gettime_nsec_np(libc::CLOCK_UPTIME_RAW) } / 1000) as i64
}

/// Moves the calling thread to user-interactive QoS. Main's bridge thread
/// runs there so a surface import never waits behind background work.
#[cfg(target_os = "macos")]
pub fn set_thread_user_interactive() {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos: u32, relative: i32) -> i32;
    }
    // QOS_CLASS_USER_INTERACTIVE. SAFETY: affects only the calling thread.
    unsafe { pthread_set_qos_class_self_np(0x21, 0) };
}
