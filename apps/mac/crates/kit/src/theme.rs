//! Semantic tokens from `main:packages/tokens/tokens.css`.
//! Native dark surfaces use neutral grey as requested for the macOS rewrite.
use gpui::{Hsla, Rgba, Window, WindowAppearance, rgb, rgba};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlSize {
    Regular,
    #[default]
    Small,
    Mini,
}
impl ControlSize {
    pub fn height(self) -> f32 {
        match self {
            Self::Regular => 28.,
            Self::Small => 24.,
            Self::Mini => 20.,
        }
    }
    pub fn font_size(self) -> f32 {
        match self {
            Self::Regular => 13.,
            Self::Small => 12.,
            Self::Mini => 11.,
        }
    }
    pub fn radius(self) -> f32 {
        match self {
            Self::Regular => 8.,
            Self::Small => 6.,
            Self::Mini => 4.,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub window: Rgba,
    pub rail: Rgba,
    pub panel: Rgba,
    pub panel_2: Rgba,
    pub sunken: Rgba,
    pub row: Rgba,
    pub row_hi: Rgba,
    pub field: Rgba,
    pub float: Rgba,
    pub ink_1: Rgba,
    pub ink_2: Rgba,
    pub ink_3: Rgba,
    pub line: Rgba,
    pub line_2: Rgba,
    pub line_strong: Rgba,
    pub section_line: Rgba,
    pub text: Rgba,
    pub text_2: Rgba,
    pub text_3: Rgba,
    pub text_4: Rgba,
    pub label: Rgba,
    pub value: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_dim: Rgba,
    pub accent_text: Rgba,
    pub on_accent: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub success: Rgba,
    pub blue: Rgba,
    pub blue_deep: Rgba,
    pub cream: Rgba,
    pub sand: Rgba,
    pub violet: Rgba,
    pub gray: Rgba,
}
impl Theme {
    pub fn for_window(appearance: Appearance, window: &Window) -> Self {
        match appearance {
            Appearance::Light => Self::light(),
            Appearance::Dark => Self::dark(),
            Appearance::System => match window.appearance() {
                WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
                _ => Self::light(),
            },
        }
    }
    pub fn light() -> Self {
        Self {
            window: rgb(0xEEEEF1),
            rail: rgb(0xF3F3F6),
            panel: rgb(0xFDFDFE),
            panel_2: rgb(0xF5F5F7),
            sunken: rgb(0xE6E6EA),
            row: rgb(0xF3F3F6),
            row_hi: rgb(0xEBEBEF),
            field: rgb(0xF3F3F6),
            float: rgb(0xFFFFFF),
            ink_1: rgba(0x14161C0B),
            ink_2: rgba(0x14161C14),
            ink_3: rgba(0x14161C29),
            line: rgba(0x14161C12),
            line_2: rgba(0x14161C1C),
            line_strong: rgba(0x14161C33),
            section_line: rgba(0x14161C17),
            text: rgb(0x1B1D23),
            text_2: rgba(0x14161CA8),
            text_3: rgba(0x14161C7A),
            text_4: rgba(0x14161C52),
            label: rgba(0x14161CCC),
            value: rgb(0x1B1D23),
            accent: rgb(0xF0580A),
            accent_hover: rgb(0xFF6A24),
            accent_dim: rgba(0xF0580A1F),
            accent_text: rgb(0xD84E06),
            on_accent: rgb(0xFFFFFF),
            danger: rgb(0xD93025),
            warning: rgb(0xA15C00),
            success: rgb(0x148F5E),
            blue: rgb(0x2B6FE3),
            blue_deep: rgb(0x2B4FD8),
            cream: rgb(0xA89B76),
            sand: rgb(0x8D8163),
            violet: rgb(0x4C5BD8),
            gray: rgb(0x6E6E76),
        }
    }
    pub fn dark() -> Self {
        Self {
            window: rgb(0x101010),
            rail: rgb(0x121212),
            panel: rgb(0x191919),
            panel_2: rgb(0x151515),
            sunken: rgb(0x121212),
            row: rgb(0x222222),
            row_hi: rgb(0x252525),
            field: rgb(0x272727),
            float: rgb(0x292929),
            ink_1: rgba(0xF2F2F20D),
            ink_2: rgba(0xF2F2F217),
            ink_3: rgba(0xF2F2F22E),
            line: rgba(0xFFFFFF0F),
            line_2: rgba(0xFFFFFF0F),
            line_strong: rgba(0xFFFFFF33),
            section_line: rgba(0xFFFFFF0F),
            text: rgb(0xF2F2F2),
            text_2: rgb(0xA5A5A5),
            text_3: rgb(0x8C8C8C),
            text_4: rgb(0x696969),
            label: rgb(0xC6C6C6),
            value: rgb(0xE2E2E2),
            accent: rgb(0xFF6019),
            accent_hover: rgb(0xFF7333),
            accent_dim: rgba(0xFF601926),
            accent_text: rgb(0xFF8A55),
            on_accent: rgb(0xFFFFFF),
            danger: rgb(0xF36357),
            warning: rgb(0xF59E0B),
            success: rgb(0x3FCF8E),
            blue: rgb(0x4C8DFF),
            blue_deep: rgb(0x3B62E8),
            cream: rgb(0xE8E2CF),
            sand: rgb(0xD8CFAF),
            violet: rgb(0x6C7BE8),
            gray: rgb(0xA9A9AE),
        }
    }
}
// Compatibility for the shell spine. New controls resolve appearance per window.
pub fn window() -> Rgba {
    Theme::dark().window
}
pub fn panel() -> Rgba {
    Theme::dark().panel
}
pub fn sunken() -> Rgba {
    Theme::dark().sunken
}
pub fn row_hi() -> Rgba {
    Theme::dark().row_hi
}
pub fn field() -> Rgba {
    Theme::dark().field
}
pub fn accent() -> Rgba {
    Theme::dark().accent
}
pub fn accent_dim() -> Rgba {
    Theme::dark().accent_dim
}
pub fn text() -> Hsla {
    Theme::dark().text.into()
}
pub fn text_2() -> Hsla {
    Theme::dark().text_2.into()
}
pub fn text_3() -> Hsla {
    Theme::dark().text_3.into()
}
pub fn hairline() -> Hsla {
    Theme::dark().line.into()
}
pub const INSET: f32 = 8.;
pub const PANEL_RADIUS: f32 = 18.;
pub const SIDEBAR_W: f32 = 248.;
pub const INSPECTOR_W: f32 = 288.;
pub const TIMELINE_H: f32 = 248.;
pub const ROW_H: f32 = 30.;
pub const VALUE_W: f32 = 128.;
pub const HEADER_H: f32 = 32.;
pub const RADIUS_XS: f32 = 4.;
pub const RADIUS_SM: f32 = 6.;
pub const RADIUS_MD: f32 = 8.;
pub const RADIUS_LG: f32 = 12.;
pub const RADIUS_XL: f32 = 16.;
pub const GUTTER: f32 = 10.;
pub const DISABLED_OPACITY: f32 = 0.5;
pub const SYSTEM_FONT: &str = ".AppleSystemUIFont";
/// Static controls do not animate. Callers can use these durations for optional motion.
pub fn motion_duration(reduce_motion: bool, level: usize) -> std::time::Duration {
    std::time::Duration::from_millis(if reduce_motion {
        0
    } else {
        *[75, 100, 160].get(level).unwrap_or(&100)
    })
}
