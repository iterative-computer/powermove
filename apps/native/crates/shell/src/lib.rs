//! Shell contracts run headlessly; AppKit/GPUI presentation is macOS-only.
pub mod agent_launcher;
/// Settings › General › "Open projects in": Separate windows (the default,
/// owner 2026-11-25) or Tabs. The app sets it from the setting at launch and
/// whenever it changes; the next project opened follows it.
static TABS_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Projects open as tabs of the window in use (else each in a window of its own).
pub fn tabs_mode() -> bool {
    TABS_MODE.load(std::sync::atomic::Ordering::Relaxed)
}
/// Sets the mode for the projects opened from now on.
pub fn set_tabs_mode(tabs: bool) {
    TABS_MODE.store(tabs, std::sync::atomic::Ordering::Relaxed);
}
pub mod input;
pub mod drag;
pub mod agent_menu;
pub mod area;
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
/// The tokens that do not change with the appearance (sizes, text steps,
/// ink steps, fixed art colours): readable on any thread, without AppKit.
pub fn scale() -> &'static pm_design::Tokens {
    static SCALE: std::sync::OnceLock<pm_design::Tokens> = std::sync::OnceLock::new();
    SCALE.get_or_init(|| pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()))
}
pub mod menu;
pub mod menu_input;
#[cfg(target_os = "macos")]
pub mod motion;
pub mod pill;
#[cfg(target_os = "macos")]
pub mod slot_stage;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
pub mod native_control;
#[cfg(target_os = "macos")]
#[cfg(target_os = "macos")]
pub mod scopes;
pub mod session;
#[cfg(target_os = "macos")]
pub mod region_pick;
pub mod settings_window;
#[cfg(target_os = "macos")]
pub mod icons;
pub mod symbol;
pub mod telemetry;
#[cfg(target_os = "macos")]
pub mod text;
pub mod theme;
pub mod transient;
pub mod typing;
#[cfg(target_os = "macos")]
pub mod view;
pub mod welcome;
#[cfg(target_os = "macos")]
pub mod welcome_view;
pub mod workspace;
#[cfg(target_os = "macos")]
pub use macos::open_editor_tab;
pub use native::{
    composer_passes, front_restoring, restore_arranging, set_front_restoring, window_shown,
};
#[cfg(target_os = "macos")]
pub use native::agent_chooser::Spec as ChooserSpec;
#[cfg(target_os = "macos")]
pub use native::{
    bring_forward, install_menus, quit_blocker_window, set_quit_hook, show_in_tab_group,
};
pub(crate) use macos::open_settings;
#[cfg(target_os = "macos")]
pub use macos::{
    Finish, Gate, OpenUrls, left_button_held, open_editor, run, run_gated, run_integrated,
    run_with_device, smoke, tokens,
};
#[cfg(debug_assertions)]
pub use macos::capture_app;
#[cfg(target_os = "macos")]
pub use native::{
    AgentStatus, viewer_preference, set_viewer_preference, apply_tabs_mode, editor_window_tabbing, front_window_id,
    live_window_ids, opens_in_place, project_menu_entries,
    ADD, Command, FlowchartRow, TaskStatus, Host, MenuEntry, SHAPES, accent_color, agent_menu_windows,
    capture_agent_picker, check_palette_typing, check_view3d, document_subtitle, flush_typing, history_titles, native_color, popover_frame, request_close, resize_from_top,
    send_menu_command, system_accent, viewer_slot, viewer_window, window_backdrop,
    zoom_limit,
};

#[cfg(all(test, target_os = "macos"))]
mod utility_capture_tests;

// Keep pure model and headless dock regressions available under `cargo test --lib`.
// The standalone native-window harnesses are intentionally separate.
#[cfg(test)]
extern crate self as pm_shell;
#[cfg(test)]
#[path = "../tests/models.rs"]
mod regression_models;
#[cfg(test)]
#[path = "../tests/shell2.rs"]
mod regression_shell2;
#[cfg(test)]
#[path = "../tests/shell3.rs"]
mod regression_shell3;
#[cfg(test)]
#[path = "../tests/docking_electron.rs"]
mod regression_docking_electron;
#[cfg(test)]
#[path = "../tests/docking_main.rs"]
mod regression_docking_main;
#[cfg(test)]
#[path = "../tests/docking_feel.rs"]
mod regression_docking_feel;
#[cfg(test)]
#[path = "../tests/dock_scroll.rs"]
mod regression_dock_scroll;
#[cfg(target_os = "macos")]
pub mod stage;
#[cfg(target_os = "macos")]
pub mod wave;
