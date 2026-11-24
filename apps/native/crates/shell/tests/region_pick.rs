//! Main-thread AppKit regression: a region pick ends with the editor or
//! project it was started for, and a fresh pick then works.
fn main() {
    #[cfg(target_os = "macos")]
    pm_shell::region_pick::check_region_pick();
}
