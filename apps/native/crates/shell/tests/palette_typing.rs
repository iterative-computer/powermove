//! AppKit layout and accessibility checks must run on the main thread.
fn main() {
    #[cfg(target_os = "macos")]
    pm_shell::check_palette_typing();
}
