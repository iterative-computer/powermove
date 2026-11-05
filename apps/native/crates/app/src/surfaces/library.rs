//! S005/S017/S018/S019. Native host boundary for the Opus library specification.
//!
//! Decisions (2026-11-01):
//! * N3/design ownership: copy is literal; review answers and host tokens are
//!   authoritative. All rendering is provisional.
//! * N4: no delete or direct library writer exists in this module. Archive
//!   deletion remains unavailable until the kernel supplies ArchiveReason.
//! * N7/N8: requests and responses carry IDs; IO/filtering runs on a worker;
//!   the UI polls completions and never waits on the kernel or on JavaScript.
//! * N1/N3: unsupported kernel operations refuse explicitly, never succeed.
//! * Prune rule/N6: no existing UI, extension or storage path is removed.
//!
//! Integration still required outside this lane: route File > Project Library,
//! Welcome > Show All Projects and New Project to this controller; implement
//! LibraryHost using window::LibraryWindow, window::grid_card and the native
//! factories in library_appkit.rs. Existing consumers remain until that host is
//! attached; these factories do not claim that a window is already installed.

// Each surface also builds as an independent portable verification target.
#[allow(clippy::duplicate_mod)]
#[path = "sheet_layout.rs"]
pub mod sheet_layout;

#[cfg(target_os = "macos")]
#[path = "library_appkit.rs"]
pub mod appkit;
#[path = "library_controller.rs"]
pub mod controller;
#[path = "library_form.rs"]
pub mod form;
#[path = "library_kernel.rs"]
pub mod kernel;
#[path = "library_model.rs"]
pub mod model;
#[cfg(target_os = "macos")]
#[path = "library_window.rs"]
pub mod window;

pub use controller::*;
pub use form::*;
pub use model::*;

/// Explicit integration/design requirements, inspectable without a window.
pub const GAPS: &[&str] = &[];

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
