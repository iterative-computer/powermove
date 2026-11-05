//! S025/S034/S035/S051, `design/surfaces/editing.md`.
//!
//! Rules: N3 uses the Opus spec for appearance and the tag for edit semantics;
//! N4/N6 leave all old consumers and opaque authored values intact; N7 exposes
//! stable ids and inspectable plans; N8 submits native commands asynchronously.
//! No document is owned or mutated by a view. AppKit attachment and engine
//! geometry publication are explicit host responsibilities, not simulated UI.

#[path = "editing_animators.rs"]
pub mod animators;
#[path = "editing_colour.rs"]
pub mod colour;
#[path = "editing_kernel.rs"]
pub mod kernel;
#[cfg(target_os = "macos")]
#[path = "editing_native.rs"]
pub mod native;
#[path = "editing_path.rs"]
pub mod path;
#[path = "editing_text.rs"]
pub mod text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accessibility {
    pub id: String,
    pub role: &'static str,
    pub label: String,
    pub selected: Option<bool>,
    pub enabled: bool,
}

#[path = "editing_host.rs"]
pub mod host;
pub use host::{EditingHost, InspectHost};

#[cfg(test)]
#[path = "editing_tests.rs"]
mod tests;
