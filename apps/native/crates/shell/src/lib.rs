//! Shell contracts run headlessly; AppKit/GPUI presentation is macOS-only.
pub mod agent_menu;
#[cfg(target_os = "macos")]
pub mod integration;
pub mod keyboard;
#[cfg(target_os = "macos")]
pub mod kit;
pub mod layout;
#[cfg(target_os = "macos")]
mod macos;
pub mod menu;
#[cfg(target_os = "macos")]
pub mod motion;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
pub mod native_control;
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
pub mod welcome;
#[cfg(target_os = "macos")]
pub mod welcome_view;
pub mod workspace;
#[cfg(target_os = "macos")]
pub use macos::open_editor_tab;
pub(crate) use macos::open_settings;
#[cfg(target_os = "macos")]
pub use macos::{
    Finish, Gate, OpenUrls, open_editor, run, run_gated, run_integrated, run_with_device, smoke,
    tokens,
};
#[cfg(target_os = "macos")]
pub use native::{
    ADD, Command, FlowchartRow, Host, MenuEntry, SHAPES, accent_color, agent_menu_windows, check_view3d,
    document_subtitle, native_color, popover_frame, request_close, resize_from_top, send_menu_command,
    sidebar_frame, system_accent, viewer_window, window_backdrop, zoom_limit,
};
