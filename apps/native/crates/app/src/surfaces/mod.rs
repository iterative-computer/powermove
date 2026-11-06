//! App-owned secondary surfaces. Platform adapters never execute JavaScript.
//! `surface_routes` and `first_run` attach these to menus and windows. What
//! still waits for its owner is marked here, and listed in docs/native/surfaces.md.
pub mod sheet_layout;

// S010–S012: reached from agent access and extension Settings rows.
#[allow(dead_code)]
pub mod authority;

// S006, S007, S024, S038–S040: reached from the media import lane's Project panel routes.
#[allow(dead_code)]
pub mod media;
#[allow(dead_code)]
pub mod media_appkit;
#[allow(dead_code)]
pub mod media_kernel;
#[allow(dead_code)]
pub mod media_wire;

// S001 alerts are attached; S013, S053 banners and S054 wait for extension hosting
// and the status bar's slowdown source.
#[allow(dead_code)]
pub mod recovery;
#[allow(dead_code)]
pub mod recovery_kernel;
#[allow(dead_code)]
pub mod recovery_macos;
#[allow(dead_code)]
pub mod recovery_native;
#[allow(dead_code)]
pub mod recovery_wire;

// Alerts are attached, except saving over a changed file, which needs a watcher.
// Notices, popouts and pickers wait for the shell's document tabs.
#[allow(dead_code)]
pub mod documents;

// Secure input and wheel conversion, for the Settings field host.
#[allow(dead_code)]
pub mod kit_native;
