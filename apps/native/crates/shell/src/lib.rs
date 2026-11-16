//! Shell contracts run headlessly; AppKit/GPUI presentation is macOS-only.
/// Settings › General › "Open projects in": Tabs (the default, owner and
/// co-founder 2026-11-14) or Separate windows (one project per window). The
/// app sets it from the setting at launch and whenever it changes; the next
/// project opened follows it.
static TABS_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// Projects open as tabs of the window in use (else each in a window of its own).
pub fn tabs_mode() -> bool {
    TABS_MODE.load(std::sync::atomic::Ordering::Relaxed)
}
/// Sets the mode for the projects opened from now on.
pub fn set_tabs_mode(tabs: bool) {
    TABS_MODE.store(tabs, std::sync::atomic::Ordering::Relaxed);
}
pub mod agent_menu;
#[cfg(target_os = "macos")]
pub mod ease;
#[cfg(target_os = "macos")]
pub mod grade;
#[cfg(target_os = "macos")]
pub mod integration;
pub mod keyboard;
pub mod keymap;
#[cfg(target_os = "macos")]
pub mod kit;
pub mod layout;
#[cfg(target_os = "macos")]
mod macos;
pub mod menu;
#[cfg(target_os = "macos")]
pub mod motion;
#[cfg(target_os = "macos")]
pub mod layer_stage;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
pub mod native_control;
#[cfg(target_os = "macos")]
pub mod scopes;
pub mod session;
#[cfg(target_os = "macos")]
pub mod settings_window;
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
    Finish, Gate, OpenUrls, left_button_held, open_editor, run, run_gated, run_integrated,
    run_with_device, smoke, tokens,
};
#[cfg(target_os = "macos")]
pub use native::{
    viewer_preference, set_viewer_preference, apply_tabs_mode, editor_window_tabbing, front_window_id,
    live_window_ids, opens_in_place, project_menu_entries,
    ADD, AgentStatus, Command, FlowchartRow, Host, MenuEntry, SHAPES, accent_color, agent_menu_windows,
    check_view3d, document_subtitle, native_color, popover_frame, request_close, resize_from_top,
    send_menu_command, sidebar_frame, system_accent, viewer_slot, viewer_window, window_backdrop,
    zoom_limit,
};

#[cfg(all(test, target_os = "macos"))]
mod utility_capture_tests;
