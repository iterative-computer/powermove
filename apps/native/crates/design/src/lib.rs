//! Powermove Native's only styling source. Theme translation targets these semantic names.
//!
//! The language these values encode is `docs/native/design/language.md`; the
//! generated table in `docs/native/design/tokens.md` is checked against this file
//! by `tests/spec.rs`. Change a value here, then regenerate the table with
//! `PM_UPDATE_DESIGN_SNAPSHOT=1 cargo test -p pm-design`.
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
    /// Accent used as text or a thin mark on panels; meets 4.5:1 on `panel`.
    pub accent_text: u32,
    /// Opaque accent tint over `panel`: drop targets, the active snap guide bed.
    pub accent_wash: u32,
    pub danger: u32,
    pub warning: u32,
    pub success: u32,
    /// Badges drawn over media (duration, offline): always dark, drawn at
    /// `metrics.media_badge_opacity`.
    pub media_badge: u32,
    /// Red channel scope trace; data colour drawn with dark-scheme viewer tokens.
    pub scope_red: u32,
    /// Green channel scope trace; data colour drawn with dark-scheme viewer tokens.
    pub scope_green: u32,
    /// Blue channel scope trace; data colour drawn with dark-scheme viewer tokens.
    pub scope_blue: u32,
    /// Clip and layer label fills. Text on them is `text`.
    pub labels: Labels,
}
/// Eight label colours, in menu order. Fills are muted so artwork stays the loudest thing on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labels {
    pub blue: u32,
    pub violet: u32,
    pub orange: u32,
    pub red: u32,
    pub yellow: u32,
    pub green: u32,
    pub teal: u32,
    pub gray: u32,
}
impl Labels {
    pub const NAMES: [&'static str; 8] = [
        "blue", "violet", "orange", "red", "yellow", "green", "teal", "gray",
    ];
    pub fn all(&self) -> [u32; 8] {
        [
            self.blue,
            self.violet,
            self.orange,
            self.red,
            self.yellow,
            self.green,
            self.teal,
            self.gray,
        ]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeScale {
    pub minimum: f32,
    pub body: f32,
    pub label: f32,
    pub title: f32,
    pub display: f32,
    /// Running prose: agent replies, sheet explanations, Settings descriptions.
    pub reading: f32,
    /// Sheet, Settings pane and empty-state headings.
    pub heading: f32,
    /// Transport and timeline timecode (SF Pro, tabular figures, medium).
    pub timecode: f32,
    pub regular_weight: f32,
    pub medium_weight: f32,
    pub semibold_weight: f32,
    /// Line height multipliers. Leading is a multiplier, never a point size.
    pub dense_leading: f32,
    pub reading_leading: f32,
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
    /// Inner radius for a control nested in a control (segment in a segmented control).
    pub inner_radius: f32,
    /// Inspector value wells: height inside a `row`, and the shared column width.
    pub field: f32,
    pub value_column: f32,
    /// Keyframe-toggle diamond and its column.
    pub keyframe: f32,
    /// Timeline geometry.
    pub track_row: f32,
    pub property_row: f32,
    pub ruler: f32,
    pub layer_column: f32,
    pub playhead: f32,
    /// SF Symbol point sizes: toolbar items, panel header buttons, inline row glyphs.
    pub symbol_toolbar: f32,
    pub symbol_panel: f32,
    pub symbol_inline: f32,
    /// Minimum pointer target. Smaller glyphs get padded hit areas.
    pub hit: f32,
    pub popover_width: f32,
    pub sheet_width: f32,
    pub agent_width: f32,
    /// Indent per outline level.
    pub tree_indent: f32,
    /// Layer-kind badge square (timeline, Project list).
    pub kind_badge: f32,
    /// Minimum Project grid thumbnail width.
    pub thumbnail_min: f32,
    /// Content inset inside sheets and Settings panes.
    pub sheet_inset: f32,
    /// Shared value column in sheets.
    pub sheet_value_column: f32,
    /// Width of wide sheets (Export, Expression).
    pub sheet_width_wide: f32,
    /// Agent composer grows to this many lines, then scrolls.
    pub composer_max_lines: f32,
    /// Settings grouped-form card corner (control_radius + inner_radius).
    pub group_radius: f32,
    /// Minimum Settings row height (row + inset).
    pub form_row: f32,
    /// Space above each Settings section after the first.
    pub section_gap: f32,
    /// Transform handle and path vertex square.
    pub gizmo_handle: f32,
    /// Gizmo stroke and keyline width over the image.
    pub gizmo_keyline: f32,
    /// Rotation handle stem length.
    pub rotation_stem: f32,
    /// Anchor point crosshair circle.
    pub anchor_mark: f32,
    /// Bézier tangent end circle.
    pub tangent_end: f32,
    /// Transparency checkerboard square.
    pub checker: f32,
    /// Opacity of selected layers drawn outside the frame.
    pub pasteboard_opacity: f32,
    /// Attention dot and drop insertion-line end circle.
    pub status_dot: f32,
    /// Vertical text padding in bubbles, the composer and code blocks.
    pub text_padding: f32,
    /// Drop-target inner stroke (2 under Increase Contrast).
    pub drop_stroke: f32,
    /// Opacity of a drag source left in place.
    pub drag_opacity: f32,
    /// Opacity of `colors.media_badge` fills over media.
    pub media_badge_opacity: f32,
    /// Mixer channel strip width.
    pub channel_strip: f32,
    /// Mixer fader knob height.
    pub fader_knob: f32,
    /// Welcome window width.
    pub welcome_width: f32,
    /// Welcome window height.
    pub welcome_height: f32,
    /// Width of In and Out brackets on rulers and scrubbers.
    pub mark_bracket: f32,
    /// Linked-clip sync badge height.
    pub sync_badge: f32,
    /// Minimum viewer width for labels in the 3D toolbar.
    pub viewer_3d_labels: f32,
    /// Viewer width below which the 3D toolbar becomes one pull-down.
    pub viewer_3d_collapse: f32,
}
/// The translucent shell: a heavy tint over a behind-window blur, one shared
/// panel material and hairline edges in `text` ink (alphas, 0–1). Reduce
/// Transparency makes the window and panels solid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Translucency {
    /// Tint over the blur. Darker than `colors.window` in dark, because the
    /// blur lightens the ground: the composite lands near the solid window and
    /// the panel material keeps its step above it (language §3, tonal elevation).
    pub ground: u32,
    pub window: f32,
    pub panel: f32,
    pub edge: f32,
    pub line: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub enter: f64,
    pub bounce: f64,
    pub exit: f64,
    pub reduced: f64,
    pub start_scale: f64,
    /// Hover, press and selection feedback.
    pub interaction: f64,
    /// Pointer rest before the scroll wheel edits a hovered value (s).
    pub wheel_rest: f64,
    /// Still time before the Focus overlay capsule fades (s).
    pub overlay_idle: f64,
    /// Delay before a tooltip appears for hover or keyboard focus (s).
    pub tooltip_delay: f64,
    /// Time window for an immediate tooltip swap to a neighbouring control (s).
    pub tooltip_warm: f64,
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
    /// Transient popovers anchored to a control (colour, easing, zoom).
    Popover,
    /// Small floating controls over the viewer image (zoom, safe areas, 3D view).
    ViewerOverlay,
    /// Agent conversation: dense text, so solid like every work surface.
    AgentPanel,
    StatusBar,
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
                | Self::Popover
                | Self::ViewerOverlay
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
    pub translucency: Translucency,
}
pub const TEXT_FONT: &str = ".SystemUIFont";
pub const DISPLAY_FONT: &str = "SF Pro Display";
pub const MONO_FONT: &str = "SF Mono";

/// A theme patch named a field pm-design does not have, or gave it an unusable value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchError(pub String);

macro_rules! patchable {
    ($($group:ident . $field:ident : $kind:ident),* $(,)?) => {
        /// Every field a theme may patch, as `group.field`. The JS theme pipeline's
        /// `native` targets must be a subset of this list (see `tests/spec.rs`).
        pub const PATCHABLE: &[&str] = &[$(concat!(stringify!($group), ".", stringify!($field))),*];
        /// Current value of a patchable field (`"metrics.gap"` → 6), as the theme
        /// pipeline's `environment.nativeTokens` expects. `None` for other names.
        pub fn field(&self, name: &str) -> Option<f64> {
            match name {
                $(concat!(stringify!($group), ".", stringify!($field)) => Some(self.$group.$field as f64),)*
                _ => None,
            }
        }
        fn set_field(&mut self, name: &str, value: f64) -> Result<(), PatchError> {
            match name {
                $(concat!(stringify!($group), ".", stringify!($field)) => {
                    patchable!(@set self.$group.$field, $kind, value, name)
                })*
                _ => Err(PatchError(format!("unknown token {name}"))),
            }
        }
    };
    (@set $target:expr, color, $value:ident, $name:ident) => {{
        if !((0. ..=16_777_215.).contains(&$value) && $value.fract() == 0.) {
            return Err(PatchError(format!("{} needs an opaque 0xRRGGBB colour", $name)));
        }
        $target = $value as u32;
        Ok(())
    }};
    (@set $target:expr, size, $value:ident, $name:ident) => {{
        if !($value.is_finite() && $value >= 0.) {
            return Err(PatchError(format!("{} needs a finite, non-negative size", $name)));
        }
        $target = $value as f32;
        Ok(())
    }};
}
impl Tokens {
    pub fn new(scheme: Scheme, accessibility: Accessibility) -> Self {
        let colors = match scheme {
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
                accent_text: 0xff8a55,
                accent_wash: 0x3c241a,
                danger: 0xf36357,
                warning: 0xf59e0b,
                success: 0x3fcf8e,
                media_badge: 0x000000,
                scope_red: 0xff6b62,
                scope_green: 0x4fd58a,
                scope_blue: 0x6ba6ff,
                labels: Labels {
                    blue: 0x3b4250,
                    violet: 0x4a3b6b,
                    orange: 0x7a3a1f,
                    red: 0x6e2b2b,
                    yellow: 0x5c4a17,
                    green: 0x24503a,
                    teal: 0x1f4b52,
                    gray: 0x3a3a3a,
                },
            },
            Scheme::Light => Colors {
                window: 0xeeeef1,
                panel: 0xfdfdfe,
                panel_secondary: 0xf5f5f7,
                viewer: 0xe4e4e4,
                field: 0xf3f3f6,
                floating: 0xffffff,
                hover: 0xebebef,
                separator: 0xd4d4d8,
                text: 0x1b1d23,
                secondary: 0x52545a,
                muted: 0x62646a,
                accent: 0xf0580a,
                selected_text: 0xffffff,
                selected_fill: 0xd5d5da,
                focus: 0xb93800,
                accent_text: 0xb93800,
                accent_wash: 0xfbe9e1,
                danger: 0xc22a1f,
                warning: 0xa15c00,
                success: 0x117a52,
                media_badge: 0x000000,
                scope_red: 0xff6b62,
                scope_green: 0x4fd58a,
                scope_blue: 0x6ba6ff,
                labels: Labels {
                    blue: 0xc9d4e8,
                    violet: 0xd8cff0,
                    orange: 0xf7cfb6,
                    red: 0xf2cac7,
                    yellow: 0xefe0b2,
                    green: 0xc5e3d1,
                    teal: 0xc2e2e3,
                    gray: 0xdcdce0,
                },
            },
        };
        let mut tokens = Self {
            scheme,
            accessibility,
            colors,
            metrics: Metrics {
                gap: 6.,
                inset: 12.,
                panel_radius: 8.,
                control_radius: 6.,
                capsule_radius: 17.,
                header: 38.,
                row: 28.,
                status: 24.,
                toolbar: 52.,
                toolbar_title_width: 160.,
                toolbar_title_line: 14.,
                control: 28.,
                border: 1.,
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
                inner_radius: 4.,
                field: 22.,
                value_column: 128.,
                keyframe: 9.,
                track_row: 28.,
                property_row: 22.,
                ruler: 24.,
                layer_column: 266.,
                playhead: 1.5,
                symbol_toolbar: 15.,
                symbol_panel: 13.,
                symbol_inline: 11.,
                hit: 24.,
                popover_width: 260.,
                sheet_width: 480.,
                agent_width: 360.,
                tree_indent: 16.,
                kind_badge: 18.,
                thumbnail_min: 120.,
                sheet_inset: 20.,
                sheet_value_column: 240.,
                sheet_width_wide: 600.,
                composer_max_lines: 10.,
                group_radius: 10.,
                form_row: 40.,
                section_gap: 40.,
                gizmo_handle: 7.,
                gizmo_keyline: 1.,
                rotation_stem: 18.,
                anchor_mark: 11.,
                tangent_end: 6.,
                checker: 8.,
                pasteboard_opacity: 0.35,
                status_dot: 6.,
                text_padding: 8.,
                drop_stroke: if accessibility.increase_contrast {
                    2.
                } else {
                    1.5
                },
                drag_opacity: 0.5,
                media_badge_opacity: 0.55,
                channel_strip: 72.,
                fader_knob: 14.,
                welcome_width: 900.,
                welcome_height: 640.,
                mark_bracket: 6.,
                sync_badge: 16.,
                viewer_3d_labels: 860.,
                viewer_3d_collapse: 560.,
            },
            typography: TypeScale {
                minimum: 11.,
                body: 12.,
                label: 12.,
                title: 13.,
                display: 22.,
                reading: 13.,
                heading: 15.,
                timecode: 13.,
                regular_weight: 400.,
                medium_weight: 500.,
                semibold_weight: 600.,
                dense_leading: 1.3,
                reading_leading: 1.45,
                text_optical_size: 12.,
                display_optical_size: 22.,
            },
            motion: Motion {
                enter: 0.32,
                bounce: 0.15,
                exit: 0.12,
                reduced: 0.15,
                start_scale: 0.96,
                interaction: 0.075,
                wheel_rest: 0.5,
                overlay_idle: 2.,
                tooltip_delay: 0.38,
                tooltip_warm: 0.6,
            },
            translucency: match (scheme, accessibility.reduce_transparency) {
                (Scheme::Dark, true) => Translucency {
                    ground: 0x111111,
                    window: 1.,
                    panel: 1.,
                    edge: 0.06,
                    line: 0.07,
                },
                (Scheme::Light, true) => Translucency {
                    ground: 0xeeeef1,
                    window: 1.,
                    panel: 1.,
                    edge: 0.08,
                    line: 0.09,
                },
                (Scheme::Dark, false) => Translucency {
                    ground: 0x080808,
                    window: 0.82,
                    panel: 0.7,
                    edge: 0.06,
                    line: 0.07,
                },
                (Scheme::Light, false) => Translucency {
                    ground: 0xeeeef1,
                    window: 0.8,
                    panel: 0.72,
                    edge: 0.08,
                    line: 0.09,
                },
            },
        };
        tokens.enforce_accessibility();
        tokens
    }
    /// Increase Contrast always wins, including over a theme patch.
    fn enforce_accessibility(&mut self) {
        if !self.accessibility.increase_contrast {
            return;
        }
        let c = &mut self.colors;
        match self.scheme {
            Scheme::Dark => {
                c.separator = 0x888888;
                c.secondary = 0xdddddd;
                c.muted = 0xbfbfbf;
                c.focus = 0xffa073;
                c.accent_text = 0xffa073;
            }
            Scheme::Light => {
                c.separator = 0x686868;
                c.secondary = 0x303036;
                c.muted = 0x414148;
                c.focus = 0x9a2f00;
                c.accent_text = 0x9a2f00;
            }
        }
        self.metrics.border = 2.;
        self.metrics.drop_stroke = 2.;
    }
    patchable! {
        colors.window: color, colors.panel: color, colors.panel_secondary: color,
        colors.viewer: color, colors.field: color, colors.floating: color,
        colors.hover: color, colors.separator: color, colors.text: color,
        colors.secondary: color, colors.muted: color, colors.accent: color,
        colors.selected_text: color, colors.selected_fill: color, colors.focus: color,
        colors.accent_text: color, colors.accent_wash: color, colors.danger: color,
        colors.warning: color, colors.success: color,
        metrics.gap: size, metrics.inset: size, metrics.panel_radius: size,
        metrics.control_radius: size, metrics.inner_radius: size, metrics.header: size,
        metrics.row: size, metrics.control: size, metrics.field: size,
        metrics.value_column: size, metrics.border: size, metrics.track_row: size,
        typography.minimum: size, typography.body: size, typography.label: size,
        typography.title: size, typography.reading: size, typography.heading: size,
        typography.display: size,
        colors.media_badge: color,
        metrics.kind_badge: size,
        metrics.thumbnail_min: size,
        metrics.status_dot: size,
    }
    /// Apply a translated theme's `nativePatch` (field name → number). Fields are
    /// applied in order; accessibility overrides are re-applied afterwards, so a
    /// theme can never lower contrast below the Increase Contrast floor.
    pub fn apply_patch<'a>(
        &mut self,
        patch: impl IntoIterator<Item = (&'a str, f64)>,
    ) -> Result<(), PatchError> {
        let mut next = self.clone();
        for (name, value) in patch {
            next.set_field(name, value)?;
        }
        next.enforce_accessibility();
        *self = next;
        Ok(())
    }
    /// Panel edge as (colour, alpha): a `text` hairline, or the solid
    /// `separator` at `metrics.border` under Increase Contrast.
    pub fn edge(&self) -> (u32, f32) {
        if self.accessibility.increase_contrast {
            (self.colors.separator, 1.)
        } else {
            (self.colors.text, self.translucency.edge)
        }
    }
    /// Divider under a panel header, between list columns and above the status line.
    pub fn line(&self) -> (u32, f32) {
        if self.accessibility.increase_contrast {
            (self.colors.separator, 1.)
        } else {
            (self.colors.text, self.translucency.line)
        }
    }
    /// Window ground as (colour, alpha): the `ground` tint over the blur, or
    /// the solid `window` colour under Reduce Transparency.
    pub fn ground(&self) -> (u32, f32) {
        if self.translucent() {
            (self.translucency.ground, self.translucency.window)
        } else {
            (self.colors.window, 1.)
        }
    }
    /// The window ground needs a behind-window blur under it.
    pub fn translucent(&self) -> bool {
        self.translucency.window < 1.
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

/// Compatibility name used by the merged timeline/media renderer.
/// The live token is metrics.media_badge_opacity (Opus panels §6).
pub mod panels {
    pub const MEDIA_BADGE_ALPHA: f64 = 0.55;
}
