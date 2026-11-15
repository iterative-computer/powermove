//! Shell contracts run headlessly; AppKit/GPUI presentation is macOS-only.
pub mod agent_launcher;
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
    Finish, Gate, OpenUrls, left_button_held, open_editor, run, run_gated, run_integrated,
    run_with_device, smoke, tokens,
};
#[cfg(target_os = "macos")]
pub use native::{
    ADD, Command, FlowchartRow, Host, MenuEntry, SHAPES, accent_color, agent_menu_windows,
    check_view3d, document_subtitle, native_color, one_project_per_window, opens_in_place,
    popover_frame, project_menu_entries, request_close, resize_from_top, send_menu_command,
    set_viewer_preference, sidebar_frame, system_accent, viewer_preference, viewer_slot,
    viewer_window, window_backdrop, zoom_limit,
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
