//! App-owned secondary surfaces. Platform adapters never execute JavaScript.
//! `surface_routes` and `first_run` attach these to menus and windows. What
//! still waits for its owner is marked here, and listed in docs/native/surfaces.md.
pub mod sheet_layout;

pub mod recovery;

// Alerts are attached, except saving over a changed file, which needs a watcher.
// Notices, popouts and pickers wait for the shell's document tabs.
pub mod documents;
