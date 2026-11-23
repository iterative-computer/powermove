//! Main-thread AppKit regression for palette typing, quick picks, and popup layout.
//! The release run also checks typing latency over 550 choices.
fn main() {
    #[cfg(target_os = "macos")]
    pm_shell::check_palette_typing();
}
