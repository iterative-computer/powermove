//! Shell's only styling source. Theme translation targets these semantic names.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scheme {
    Light,
    #[default]
    Dark,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accessibility {
    pub reduce_transparency: bool,
    pub increase_contrast: bool,
    pub reduce_motion: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Colors {
    pub window: u32,
    pub panel: u32,
    pub panel_secondary: u32,
    pub viewer: u32,
    pub field: u32,
    pub floating: u32,
    pub hover: u32,
    pub separator: u32,
    pub text: u32,
    pub secondary: u32,
    pub muted: u32,
    pub accent: u32,
    pub selected_text: u32,
    pub selected_fill: u32,
    pub focus: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeScale {
    pub minimum: f32,
    pub body: f32,
    pub label: f32,
    pub title: f32,
    pub display: f32,
    pub text_optical_size: f32,
    pub display_optical_size: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub gap: f32,
    pub inset: f32,
    pub panel_radius: f32,
    pub control_radius: f32,
    pub capsule_radius: f32,
    pub header: f32,
    pub row: f32,
    pub status: f32,
    pub toolbar: f32,
    pub toolbar_title_width: f32,
    pub toolbar_title_line: f32,
    pub control: f32,
    pub border: f32,
    pub min_panel: f32,
    pub min_center: f32,
    pub keyboard_step: f32,
    pub drag_threshold: f32,
    pub window_width: f32,
    pub window_height: f32,
    pub min_width: f32,
    pub min_height: f32,
    pub palette_width: f32,
    pub palette_height: f32,
    pub field_menu_width: f32,
    pub field_menu_height: f32,
    pub settings_width: f32,
    pub settings_height: f32,
    pub settings_sidebar: f32,
    pub menu_inset: f32,
    pub menu_row: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub enter: f64,
    pub bounce: f64,
    pub exit: f64,
    pub reduced: f64,
    pub start_scale: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Toolbar,
    FloatingBar,
    Composer,
    Menu,
    Palette,
    Sheet,
    Sidebar,
    Viewer,
    Timeline,
    InspectorRow,
    List,
    Panel,
}
impl Surface {
    pub fn permits_glass(self) -> bool {
        matches!(
            self,
            Self::Toolbar
                | Self::FloatingBar
                | Self::Composer
                | Self::Menu
                | Self::Palette
                | Self::Sheet
                | Self::Sidebar
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Solid,
    VisualEffect,
    Glass,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    pub scheme: Scheme,
    pub accessibility: Accessibility,
    pub colors: Colors,
    pub metrics: Metrics,
    pub typography: TypeScale,
    pub motion: Motion,
}
pub const TEXT_FONT: &str = ".SystemUIFont";
pub const DISPLAY_FONT: &str = "SF Pro Display";
pub const MONO_FONT: &str = "SF Mono";
impl Tokens {
    pub fn new(scheme: Scheme, accessibility: Accessibility) -> Self {
        let mut colors = match scheme {
            Scheme::Dark => Colors {
                window: 0x111111,
                panel: 0x1a1a1a,
                panel_secondary: 0x161616,
                viewer: 0x0e0e0e,
                field: 0x262626,
                floating: 0x2c2c2c,
                hover: 0x232323,
                separator: 0x303030,
                text: 0xf2f2f2,
                secondary: 0xb4b4b4,
                muted: 0x949494,
                accent: 0xff6019,
                selected_text: 0xffffff,
                selected_fill: 0x3a3a3a,
                focus: 0xff8a55,
            },
            Scheme::Light => Colors {
                window: 0xeeeef1,
                panel: 0xfdfdfe,
                panel_secondary: 0xf5f5f7,
                viewer: 0xe6e6ea,
                field: 0xf3f3f6,
                floating: 0xffffff,
                hover: 0xebebef,
                separator: 0xd4d4d8,
                text: 0x1b1d23,
                secondary: 0x52545a,
                muted: 0x62646a,
                accent: 0xff6019,
                selected_text: 0xffffff,
                selected_fill: 0xd5d5da,
                focus: 0xb93800,
            },
        };
        if accessibility.increase_contrast {
            match scheme {
                Scheme::Dark => {
                    colors.separator = 0x888888;
                    colors.secondary = 0xdddddd;
                    colors.muted = 0xbfbfbf;
                    colors.focus = 0xffa073;
                }
                Scheme::Light => {
                    colors.separator = 0x686868;
                    colors.secondary = 0x303036;
                    colors.muted = 0x414148;
                }
            }
        }
        Self {
            scheme,
            accessibility,
            colors,
            metrics: Metrics {
                gap: 6.,
                inset: 12.,
                panel_radius: 12.,
                control_radius: 6.,
                capsule_radius: 17.,
                header: 36.,
                row: 28.,
                status: 30.,
                toolbar: 52.,
                toolbar_title_width: 160.,
                toolbar_title_line: 14.,
                control: 28.,
                border: if accessibility.increase_contrast {
                    2.
                } else {
                    1.
                },
                min_panel: 72.,
                min_center: 180.,
                keyboard_step: 10.,
                drag_threshold: 3.,
                window_width: 1440.,
                window_height: 900.,
                min_width: 980.,
                min_height: 640.,
                palette_width: 520.,
                palette_height: 360.,
                field_menu_width: 300.,
                field_menu_height: 280.,
                settings_width: 760.,
                settings_height: 520.,
                settings_sidebar: 180.,
                menu_inset: 12.,
                menu_row: 30.,
            },
            typography: TypeScale {
                minimum: 11.,
                body: 12.,
                label: 12.,
                title: 13.,
                display: 22.,
                text_optical_size: 12.,
                display_optical_size: 22.,
            },
            motion: Motion {
                enter: 0.32,
                bounce: 0.15,
                exit: 0.12,
                reduced: 0.15,
                start_scale: 0.96,
            },
        }
    }
    pub fn material(&self, surface: Surface, glass_available: bool) -> Material {
        if !surface.permits_glass() || self.accessibility.reduce_transparency {
            Material::Solid
        } else if glass_available {
            Material::Glass
        } else {
            Material::VisualEffect
        }
    }
}
/// Persisted dimensions are workspace data, not styling literals in views.
pub mod workspace {
    pub const DEFAULT_LEFT: f32 = 300.;
    pub const DEFAULT_RIGHT: f32 = 300.;
    pub const DEFAULT_TIMELINE: f32 = 344.;
    pub const DEFAULT_EFFECTS: f32 = 268.;
}
