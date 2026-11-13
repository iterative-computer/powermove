//! Powermove Native's only styling source. Theme translation targets these semantic names.
//!
//! The system these values encode is `docs/native/design/language.md`; the
//! generated table in `docs/native/design/tokens.md` is checked against this file
//! by `tests/spec.rs`. Change a value here, then regenerate the table with
//! `PM_UPDATE_DESIGN_SNAPSHOT=1 cargo test -p pm-design`.
//!
//! One ink per appearance and a handful of OKLCH roles make every colour
//! ([`Colors::of`]); sizes come from one spacing scale and one radius scale
//! ([`Space`], [`Radii`]); text, edges and elevation are recipes ([`Text`],
//! [`Elevation`]); motion is springs ([`motion`]); the timeline's strips are a
//! drawn material ([`Strip`]).
pub mod color;
pub mod motion;
mod recipe;
mod strip;
pub use color::mix;
use color::oklch;
pub use recipe::{Elevation, Shadow, Text, TextStyle};
use serde::{Deserialize, Serialize};
pub use strip::{GLOW_FROM, GRID_FADE, Playhead, Strip, StripState, TrimGrip};
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
    /// Third text step: placeholders, shortcut hints, disabled labels and
    /// counts nobody has to read (`text` ink at `translucency.faint`).
    pub faint: u32,
    /// The raised neutral surface: the selected segment or tab, a neutral
    /// push button, a card. Drawn with `Elevation::Raised`. Never the accent.
    pub raised: u32,
    pub accent: u32,
    pub selected_text: u32,
    pub selected_fill: u32,
    /// Soft neutral fill: segmented tracks, search fields, quiet wells
    /// (`text` ink at `translucency.hover` over `panel`).
    pub fill: u32,
    /// Neutral on state: the selected tab or segment, a pressed toggle
    /// (`text` ink at `translucency.selected` over `panel`). Never the accent.
    pub fill_strong: u32,
    /// Opaque hairline for control and card edges (`text` ink at
    /// `translucency.edge` over `panel`; `separator` under Increase Contrast).
    pub hairline: u32,
    /// Metadata on a selected row (`selected_fill`): ≥ 4.5:1 where `muted` is not.
    pub selected_secondary: u32,
    /// `danger` words on a selected row (`Missing`): ≥ 4.5:1 on `selected_fill`.
    pub selected_danger: u32,
    /// Keyboard focus ring on GPUI surfaces: neutral, never the accent (owner rule).
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
    /// The viewer's ruler guides: a cyan (the accent marks selection handles
    /// there), deeper in the light appearance, where they also cross the
    /// light surround.
    pub viewer_guide: u32,
    /// The viewer's grid: a light neutral over the picture, drawn at
    /// `translucency.line`.
    pub viewer_grid: u32,
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
    /// The emphasis weight panels use for their titles and section headings:
    /// medium, the kit's Title weight (weight is not the hierarchy).
    pub semibold_weight: f32,
    /// Heading and Display, the only semibold text.
    pub display_weight: f32,
    /// Line height multipliers. Leading is a multiplier, never a point size.
    pub dense_leading: f32,
    pub reading_leading: f32,
    pub text_optical_size: f32,
    pub display_optical_size: f32,
    /// Tracking (pt) of `heading` and `display`: large type is set tighter.
    /// Text sizes use the system font's own tracking table.
    pub heading_tracking: f32,
    pub display_tracking: f32,
}
/// The spacing scale. Every gap, padding and inset is one of these steps.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub hair: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
    pub section: f32,
}
/// The radius scale: multiples of `base`. A nested corner is its parent's
/// radius minus the inset between them, and never below `sm`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Radii {
    pub base: f32,
    /// Checkbox, kind badge, keyline details (base).
    pub xs: f32,
    /// Wells, segments, menu rows: the smallest corner a control gets (base × 1.5).
    pub sm: f32,
    /// Controls: buttons, fields, tabs, pill tracks (base × 2).
    pub md: f32,
    /// Panels, cards, menus (base × 3).
    pub lg: f32,
    /// Popovers, sheets, the palette (base × 4).
    pub xl: f32,
    /// The window's own corner on macOS 26 and later (system-owned).
    pub window: f32,
    /// Continuous-corner smoothing for `lg` and larger surfaces (0–1). Small
    /// controls stay circular so nested corners match.
    pub smoothing: f32,
}
impl Radii {
    fn new(base: f32) -> Self {
        Self {
            base,
            xs: base,
            sm: base * 1.5,
            md: base * 2.,
            lg: base * 3.,
            xl: base * 4.,
            window: 26.,
            smoothing: 0.6,
        }
    }
    /// The corner of a surface nested `inset` inside one of radius `outer`.
    pub fn nested(&self, outer: f32, inset: f32) -> f32 {
        (outer - inset).max(self.sm)
    }
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
    /// A plain window's title bar band (Settings, Export): content starts below it.
    pub titlebar: f32,
    pub toolbar_title_width: f32,
    pub toolbar_title_line: f32,
    pub control: f32,
    /// Space between docked panels, and between them and the window edge.
    pub dock_gap: f32,
    /// Sidebar width of full-window screens (Home, Settings).
    pub sidebar: f32,
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
    /// Text tabs and full-height segmented controls in sheets and Settings.
    pub tab: f32,
    /// Pill track of a segmented control or tab strip; its segments are
    /// concentric inside it (`pill_radius − pill_inset`).
    pub pill_radius: f32,
    /// Padding between a pill track and its segments, and between segments.
    pub pill_inset: f32,
    /// Segment and tab corner inside a pill: `pill_radius − pill_inset`.
    pub segment_radius: f32,
    /// Inspector value wells: height inside a `row`, and the least width of
    /// a row's value column (it takes what the label column leaves).
    pub field: f32,
    pub value_column: f32,
    /// The most a row's label takes before its value; it gives way first
    /// when the panel is too narrow for both.
    pub label_column: f32,
    /// Keyframe-toggle diamond and its column.
    pub keyframe: f32,
    /// Timeline geometry.
    pub track_row: f32,
    pub property_row: f32,
    pub ruler: f32,
    /// The viewer's rulers: a band along the top and the leading edge.
    pub viewer_ruler: f32,
    /// A colour wheel's diameter (Lift, Gamma, Gain, Offset).
    pub grade_wheel: f32,
    /// The curve editor's graph height.
    pub curve_editor: f32,
    pub layer_column: f32,
    /// Timeline clip bar corners (v1–3's `chrome.timeline.clipRadius`).
    pub clip_radius: f32,
    pub playhead: f32,
    /// SF Symbol point sizes: toolbar items, panel header buttons, inline row glyphs.
    pub symbol_toolbar: f32,
    pub symbol_panel: f32,
    pub symbol_inline: f32,
    /// A surface's one primary action drawn as a glyph (a composer's Send).
    pub symbol_action: f32,
    /// Minimum pointer target. Smaller glyphs get padded hit areas.
    pub hit: f32,
    pub popover_width: f32,
    /// Corner of floating popovers drawn by Powermove (the agent popover).
    pub popover_radius: f32,
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
    /// Tallest a sheet's option body grows before it scrolls, so the sheet
    /// and its footer stay on screen.
    pub sheet_body_max: f32,
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
    /// Timeline ruler ticks and the time grid under the tracks, as `text` ink.
    pub tick_minor: f32,
    pub tick_major: f32,
    pub grid: f32,
    /// Media Offline hatch on timeline clips, as `danger` ink.
    pub offline_hatch: f32,
    /// Hover wash under the pointer and soft fills, as `text` ink.
    pub hover: f32,
    /// Neutral on state (selected tab or segment), as `text` ink.
    pub selected: f32,
    /// Pressed wash, as `text` ink.
    pub pressed: f32,
    /// Third text step (`colors.faint`), as `text` ink.
    pub faint: f32,
    /// Tint of the window colour on system glass that carries labels (a
    /// sidebar), so text keeps its contrast over any desktop.
    pub glass_tint: f32,
    /// The floating colour laid under the rows of a palette or menu on
    /// system glass. The glass keeps the rim and shows faintly through; the
    /// rows keep their contrast over any content (a dark viewer in light).
    pub glass_menu: f32,
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
    /// How long a status note (`Exported …`) holds before the status line
    /// returns (s); errors stay until replaced.
    pub note_hold: f64,
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
    pub space: Space,
    pub radius: Radii,
    pub typography: TypeScale,
    pub motion: Motion,
    pub translucency: Translucency,
}
/// Layer colours, dark then light: the body of a kind badge and the hue a
/// strip takes. Text on them is `text`.
const LABELS: [Labels; 2] = [
    Labels {
        blue: 0x3b4250,
        violet: 0x4a3b6b,
        orange: 0x7a3a1f,
        red: 0x6e2b2b,
        yellow: 0x5c4a17,
        green: 0x24503a,
        teal: 0x1f4b52,
        gray: 0x3a3a3a,
    },
    Labels {
        blue: 0xc9d4e8,
        violet: 0xd8cff0,
        orange: 0xf7cfb6,
        red: 0xf2cac7,
        yellow: 0xefe0b2,
        green: 0xc5e3d1,
        teal: 0xc2e2e3,
        gray: 0xdcdce0,
    },
];
impl Colors {
    /// The colour table of one appearance. Seven roles are chosen in OKLCH: an
    /// ink, four grounds (window, panel, recess, viewer), a floating surface
    /// and the accent. Every other neutral is the ink over a ground at an
    /// alpha step, raised where needed to clear its contrast floor.
    ///
    /// Dark is warm stone (hue about 55°, chroma under 0.01): the grounds step
    /// up in lightness toward the viewer's surround, so elevation is tonal.
    /// Light is a cool paper (hue 286°). The accent is Powermove orange; it
    /// marks one action or one state per view and is never a selected fill.
    pub fn of(scheme: Scheme) -> Self {
        let dark = scheme == Scheme::Dark;
        let (ink, window, panel, recess, viewer, floating) = if dark {
            (
                oklch(0.97, 0.0013, 106.),
                oklch(0.147, 0.004, 49.),
                oklch(0.216, 0.006, 56.),
                oklch(0.188, 0.005, 67.),
                oklch(0.135, 0.003, 70.),
                oklch(0.268, 0.006, 34.),
            )
        } else {
            (
                oklch(0.201, 0.0124, 270.7),
                oklch(0.95, 0.004, 286.),
                oklch(0.994, 0.0013, 286.),
                oklch(0.971, 0.0027, 286.),
                oklch(0.926, 0.0054, 286.),
                0xffffff,
            )
        };
        // Ink alpha steps: text, secondary, muted; hover and selected fills.
        let (text, secondary, muted, wash, selected, line) = if dark {
            (1., 0.7, 0.54, 0.055, 0.12, 0.12)
        } else {
            (0.96, 0.74, 0.62, 0.045, 0.1, 0.14)
        };
        let field = mix(panel, ink, wash);
        let selected_fill = mix(panel, ink, selected);
        let grounds = [window, panel, field, floating, selected_fill];
        let secondary = color::ink_step(panel, ink, secondary, 4.5, &grounds);
        Self {
            window,
            panel,
            panel_secondary: recess,
            viewer,
            field,
            floating,
            hover: mix(panel, ink, wash + 0.03),
            separator: mix(panel, ink, line),
            text: mix(panel, ink, text),
            secondary,
            muted: color::ink_step(panel, ink, muted, 4.5, &grounds),
            faint: 0,
            raised: 0,
            accent: if dark {
                // A step under v1's #FF6B1A so a white label on it clears 3:1.
                oklch(0.685, 0.2, 42.5)
            } else {
                oklch(0.656, 0.2, 40.5)
            },
            selected_text: 0xffffff,
            selected_fill,
            fill: 0,
            fill_strong: 0,
            hairline: 0,
            selected_secondary: secondary,
            selected_danger: if dark { 0xff8a80 } else { 0xa82319 },
            focus: mix(panel, ink, if dark { 0.58 } else { 0.5 }),
            accent_text: if dark {
                oklch(0.753, 0.16, 47.)
            } else {
                oklch(0.527, 0.1735, 37.8)
            },
            accent_wash: mix(
                panel,
                if dark { 0xff6b1a } else { 0xf0580a },
                if dark { 0.16 } else { 0.1 },
            ),
            danger: if dark {
                oklch(0.68, 0.18, 27.6)
            } else {
                oklch(0.532, 0.19, 29.3)
            },
            warning: if dark {
                oklch(0.769, 0.165, 70.)
            } else {
                oklch(0.542, 0.124, 62.5)
            },
            success: if dark {
                oklch(0.763, 0.154, 159.)
            } else {
                oklch(0.514, 0.11, 161.)
            },
            media_badge: 0x000000,
            // Over a light surround a lighter cyan fades; a deeper one
            // still reads over a dark picture.
            viewer_guide: if dark {
                oklch(0.78, 0.13, 215.)
            } else {
                oklch(0.62, 0.13, 220.)
            },
            viewer_grid: 0xd8d8d8,
            scope_red: 0xff6b62,
            scope_green: 0x4fd58a,
            scope_blue: 0x6ba6ff,
            labels: LABELS[usize::from(!dark)],
        }
    }
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
        let colors = Colors::of(scheme);
        let ground = colors.window;
        let space = Space {
            hair: 2.,
            xs: 4.,
            sm: 6.,
            md: 8.,
            lg: 12.,
            xl: 16.,
            xxl: 24.,
            section: 32.,
        };
        let radius = Radii::new(4.);
        let mut tokens = Self {
            scheme,
            accessibility,
            colors,
            space,
            radius,
            metrics: Metrics {
                gap: space.sm,
                inset: space.lg,
                panel_radius: radius.lg,
                control_radius: radius.md,
                capsule_radius: 17.,
                header: 38.,
                row: 28.,
                status: 24.,
                toolbar: 52.,
                titlebar: 28.,
                toolbar_title_width: 160.,
                toolbar_title_line: 14.,
                control: 28.,
                dock_gap: space.md,
                sidebar: 224.,
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
                settings_sidebar: 200.,
                menu_inset: 12.,
                menu_row: 30.,
                inner_radius: radius.sm,
                tab: 28.,
                pill_radius: radius.md,
                pill_inset: space.hair,
                segment_radius: radius.nested(radius.md, space.hair),
                field: 22.,
                value_column: 128.,
                label_column: 120.,
                keyframe: 9.,
                // v1's row height: the strip is 26 pt, tall enough for its
                // material to read, with no density lost against v1.
                track_row: 30.,
                property_row: 22.,
                ruler: 24.,
                viewer_ruler: 16.,
                grade_wheel: 112.,
                curve_editor: 200.,
                layer_column: 320.,
                clip_radius: radius.sm,
                playhead: 1.,
                symbol_toolbar: 14.,
                symbol_panel: 14.,
                symbol_inline: 12.,
                symbol_action: 20.,
                hit: 24.,
                popover_width: 260.,
                popover_radius: radius.xl,
                sheet_width: 480.,
                agent_width: 360.,
                tree_indent: 16.,
                kind_badge: 18.,
                thumbnail_min: 120.,
                sheet_inset: 20.,
                sheet_value_column: 240.,
                sheet_width_wide: 600.,
                sheet_body_max: 440.,
                composer_max_lines: 10.,
                group_radius: radius.lg,
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
                display: 24.,
                reading: 13.,
                heading: 15.,
                timecode: 13.,
                regular_weight: 400.,
                medium_weight: 500.,
                semibold_weight: 500.,
                display_weight: 600.,
                dense_leading: 1.3,
                reading_leading: 1.45,
                text_optical_size: 12.,
                display_optical_size: 24.,
                heading_tracking: -0.1,
                display_tracking: -0.2,
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
                note_hold: 2.2,
            },
            translucency: {
                // The translucent shell is a heavy tint of the window colour
                // over a behind-window blur; Reduce Transparency makes it solid.
                let solid = accessibility.reduce_transparency;
                match scheme {
                    Scheme::Dark => Translucency {
                        ground,
                        window: if solid { 1. } else { 0.9 },
                        panel: if solid { 1. } else { 0.72 },
                        edge: 0.07,
                        line: 0.07,
                        tick_minor: 0.14,
                        tick_major: 0.3,
                        grid: 0.04,
                        offline_hatch: 0.28,
                        hover: 0.06,
                        selected: 0.1,
                        pressed: 0.14,
                        faint: 0.36,
                        glass_tint: 0.9,
                        glass_menu: 0.72,
                    },
                    Scheme::Light => Translucency {
                        ground,
                        window: if solid { 1. } else { 0.62 },
                        panel: if solid { 1. } else { 0.7 },
                        edge: 0.08,
                        line: 0.08,
                        tick_minor: 0.14,
                        tick_major: 0.3,
                        grid: 0.04,
                        offline_hatch: 0.28,
                        hover: 0.045,
                        selected: 0.07,
                        pressed: 0.1,
                        faint: 0.42,
                        glass_tint: 0.6,
                        glass_menu: 0.84,
                    },
                }
            },
        };
        tokens.enforce_accessibility();
        tokens.derive_fills();
        tokens
    }
    /// Everything that is `text` ink over a ground at an alpha step follows a
    /// theme's colour patch: soft fills, the raised surface, hairlines and the
    /// third text step.
    fn derive_fills(&mut self) {
        let (ink, ground, t) = (self.colors.text, self.colors.panel, self.translucency);
        let c = &mut self.colors;
        c.fill = mix(ground, ink, t.hover);
        // The on state sits on raised tracks as well as panels.
        c.fill_strong = mix(c.floating, ink, t.selected);
        c.raised = match self.scheme {
            Scheme::Dark => mix(c.floating, ink, 0.07),
            Scheme::Light => c.floating,
        };
        c.faint = mix(ground, ink, t.faint);
        c.hairline = if self.accessibility.increase_contrast {
            c.separator
        } else {
            mix(ground, ink, t.edge)
        };
    }
    /// Increase Contrast always wins, including over a theme patch: text steps
    /// rise to 7:1 and 4.5:1 on every ground, separators to 3:1, strokes double.
    fn enforce_accessibility(&mut self) {
        if !self.accessibility.increase_contrast {
            return;
        }
        let c = &mut self.colors;
        let grounds = [c.window, c.panel, c.field, c.floating, c.selected_fill];
        c.separator = color::ink_step(c.panel, c.text, 0.46, 3., &[c.panel, c.window]);
        c.secondary = color::ink_step(c.panel, c.text, 0.86, 7., &grounds);
        c.selected_secondary = c.secondary;
        c.muted = color::ink_step(c.panel, c.text, 0.74, 4.5, &grounds);
        c.focus = mix(c.panel, c.text, 0.88);
        c.accent_text = match self.scheme {
            Scheme::Dark => 0xffa073,
            Scheme::Light => 0x9a2f00,
        };
        self.metrics.border = 2.;
        self.metrics.drop_stroke = 2.;
        // Washes and on states read at a glance under Increase Contrast.
        let t = &mut self.translucency;
        (t.hover, t.selected, t.pressed) = match self.scheme {
            Scheme::Dark => (0.1, 0.16, 0.2),
            Scheme::Light => (0.08, 0.12, 0.16),
        };
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
        metrics.tab: size, metrics.pill_radius: size, metrics.pill_inset: size,
        metrics.segment_radius: size,
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
            // v1 themes have one gap (`--gap`): it spaces the docks too.
            if name == "metrics.gap" {
                next.metrics.dock_gap = next.metrics.gap;
            }
        }
        next.enforce_accessibility();
        next.derive_fills();
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
    /// The veil over the timeline's tracks outside the work area, as
    /// (colour, alpha): the canvas's second step, so strips there fade
    /// toward the ground in either appearance. Names are painted over it.
    pub fn work_area_veil(&self) -> (u32, f32) {
        (self.colors.panel_secondary, 0.3)
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
    /// The spring for `preset` under the current accessibility settings:
    /// Reduce Motion snaps anything that moves or scales and turns fades into
    /// the `motion.reduced` cross-fade.
    pub fn spring(&self, preset: motion::Preset) -> motion::Spring {
        if !self.accessibility.reduce_motion {
            preset.spring(&self.motion)
        } else if preset.moves() {
            motion::Spring::SNAP
        } else {
            motion::Spring::settling(self.motion.reduced)
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
