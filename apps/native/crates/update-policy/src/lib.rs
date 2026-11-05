//! Shared update admission and durable staging policy. No application services,
//! migration coordinator, UI or network transport. The opt-in macOS binding
//! enqueues public Sparkle calls; production host guards remain mandatory.
#[cfg(all(target_os = "macos", feature = "macos-sparkle"))]
pub mod sparkle_native;
pub mod update_distribution;
pub mod update_handoff;
pub mod updater;
