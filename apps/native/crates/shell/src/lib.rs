//! Shell contracts run headlessly; AppKit/GPUI presentation is macOS-only.
#[cfg(target_os = "macos")]
pub mod integration;
pub mod keyboard;
pub mod layout;
#[cfg(target_os = "macos")]
mod macos;
pub mod menu;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
pub mod scopes;
pub mod session;
#[cfg(target_os = "macos")]
pub mod symbol;
#[cfg(target_os = "macos")]
pub mod text;
pub mod theme;
pub mod transient;
#[cfg(target_os = "macos")]
pub mod view;
#[cfg(target_os = "macos")]
pub(crate) use macos::{open_editor, open_editor_tab, open_settings};
#[cfg(target_os = "macos")]
pub use macos::{run, run_integrated, run_with_device, smoke};
#[cfg(target_os = "macos")]
pub use native::{
    Command, FlowchartRow, Host, MenuEntry, accent_color, document_subtitle, system_accent,
};
