//! The native font registry for S4: answers `s4_layout::font::FontResolver`
//! with CoreText and AppKit, calling the same platform APIs in the same order
//! as Blink's macOS font matching (Chromium 152, `font_matcher_mac.mm`,
//! `font_cache_mac.mm`, `font_platform_data_mac.mm`). Every answer is
//! recorded so `s4_layout::replay` can answer the same queries from font
//! data alone (the `pm-wasm` path).

#[cfg(target_os = "macos")]
mod coretext;

#[cfg(target_os = "macos")]
pub use coretext::CoreTextRegistry;
