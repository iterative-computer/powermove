//! Colors and metrics. Values come from packages/tokens/tokens.css (dark).

use gpui::{Hsla, Rgba, rgb, rgba};

pub fn window() -> Rgba {
    rgb(0x0F0D0D)
}
pub fn panel() -> Rgba {
    rgb(0x1A1817)
}
pub fn sunken() -> Rgba {
    rgb(0x121010)
}
pub fn row_hi() -> Rgba {
    rgb(0x252321)
}
pub fn field() -> Rgba {
    rgba(0xFFFFFF14)
}
pub fn accent() -> Rgba {
    rgb(0xFF6019)
}
pub fn accent_dim() -> Rgba {
    rgba(0xFF601926)
}
pub fn text() -> Hsla {
    gpui::white().opacity(0.92)
}
pub fn text_2() -> Hsla {
    gpui::white().opacity(0.6)
}
pub fn text_3() -> Hsla {
    gpui::white().opacity(0.38)
}
pub fn hairline() -> Hsla {
    gpui::white().opacity(0.08)
}

/// HIG: floating panels inset from the window edge, concentric corners.
pub const INSET: f32 = 8.0;
pub const PANEL_RADIUS: f32 = 18.0;
pub const SIDEBAR_W: f32 = 248.0;
pub const INSPECTOR_W: f32 = 288.0;
pub const TIMELINE_H: f32 = 248.0;
pub const ROW_H: f32 = 28.0;
