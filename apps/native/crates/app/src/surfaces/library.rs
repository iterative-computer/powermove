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
//! `crate::project_library` hosts this controller: File ▸ Projects…, File ▸ New
//! Project and the library's alerts and sheets all reach the kernel through it.

pub use crate::surfaces::sheet_layout;

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

pub use controller::*;
pub use form::*;
pub use model::*;

/// Explicit integration/design requirements, inspectable without a window.
pub const GAPS: &[&str] = &[];

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
