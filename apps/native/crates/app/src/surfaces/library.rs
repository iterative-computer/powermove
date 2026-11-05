//! S005/S017/S018/S019. Native host boundary for the Opus library specification.
//!
//! Decisions (2026-11-01):
//! * N3/design ownership: copy is literal; unresolved design tokens/semantics
//!   remain named gaps, never guessed values. All rendering is provisional.
//! * N4: no delete or direct library writer exists in this module. Archive
//!   deletion remains unavailable until the kernel supplies ArchiveReason.
//! * N7/N8: requests and responses carry IDs; IO/filtering runs on a worker;
//!   the UI polls completions and never waits on the kernel or on JavaScript.
//! * N1/N3: unsupported kernel operations refuse explicitly, never succeed.
//! * Prune rule/N6: no existing UI, extension or storage path is removed.
//!
//! Integration still required outside this lane: route File > Project Library,
//! Welcome > Show All Projects and New Project to this controller; implement
//! LibraryHost with the AppKit window/sheet and UI-kit renderer. The available
//! AppKit alert adapter is in library_appkit.rs. This is a tested controller
//! and presentation contract, not a claim that a window is installed.

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
pub const GAPS: &[&str] = &[
    "DESIGN-TODO(opus): pm-design is missing metrics.thumbnail_min, metrics.sheet_inset, metrics.sheet_value_column and colors.media_badge specified in panels/README §6; keep symbolic references until the token owner adds them.",
    "DESIGN-TODO(opus): specify missing-summary card facts/VoiceOver wording; omit unknown facts rather than invent zero values.",
    "DESIGN-TODO(opus): specify accessibility label and initial state of the New Project size-link toggle, and the loading reason/empty-state symbols.",
    "DESIGN-TODO(opus): specify the direction/mapping of Duration, Size and Frame Rate table sorts (Sort only defines Date Modified and Name).",
    "DESIGN-TODO(opus): specify drop-frame timecode entry; semicolon timecodes remain invalid rather than being interpreted at the wrong rate.",
    "DESIGN-TODO(opus): reveal/select a duplicate outside the current filter without guessing a filter change.",
    "DESIGN-TODO(opus): specify cancellation while New Project creation is already admitted; the default action stays disabled to prevent duplicate creates.",
];

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
