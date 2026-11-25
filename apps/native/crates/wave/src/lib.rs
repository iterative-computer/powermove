//! Snapshot-based distortion waves, ported from distortion-waves 0.1.0.
//! The pure timeline is independent of GPUI; `pm_shell::wave` owns live views.
mod style;
mod timeline;
pub use style::*;
pub use timeline::*;
#[cfg(target_os = "macos")]
mod renderer;
#[cfg(target_os = "macos")]
pub use renderer::*;
