//! S035's NSPopover plan. Colour conversion/notation runs in the existing v4
//! colour-space worker; authored strings remain opaque at the presentation seam.
use pm_design::{Material, Surface, Tokens};

pub const RESTORE: &str = "Restore Previous Colour";
pub const SETTINGS: &str = "Picker Settings";
pub const GAMUT_WARNING: &str = "Outside sRGB; saved as the nearest sRGB colour";
pub const EYEDROPPER: &str = "Pick a Colour From the Screen";
pub const SB_LABEL: &str = "Saturation and brightness";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillType {
    None,
    Solid,
    Linear,
    Radial,
}
impl FillType {
    pub const ALL: [Self; 4] = [Self::None, Self::Solid, Self::Linear, Self::Radial];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Solid => "Solid",
            Self::Linear => "Linear",
            Self::Radial => "Radial",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Srgb,
    DisplayP3,
}
impl Space {
    pub const ALL: [Self; 2] = [Self::Srgb, Self::DisplayP3];
    pub fn label(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB",
            Self::DisplayP3 => "Display P3",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notation {
    Hex,
    Rgb,
    Hsl,
    Oklch,
    Oklab,
    P3,
}
impl Notation {
    pub const ALL: [Self; 6] = [
        Self::Hex,
        Self::Rgb,
        Self::Hsl,
        Self::Oklch,
        Self::Oklab,
        Self::P3,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Hex => "Hex",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
            Self::Oklch => "OKLCH",
            Self::Oklab => "OKLab",
            Self::P3 => "P3",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HueMapping {
    Closest,
    Chroma,
    Lightness,
}
impl HueMapping {
    pub const ALL: [Self; 3] = [Self::Closest, Self::Chroma, Self::Lightness];
    pub fn label(self) -> &'static str {
        match self {
            Self::Closest => "Find Closest Color",
            Self::Chroma => "Maintain Chroma",
            Self::Lightness => "Maintain Lightness",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub section: &'static str,
    pub label: String,
    pub checked: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PopoverPlan {
    pub native_class: &'static str,
    pub width: f32,
    pub inset: f32,
    pub material: Material,
    pub content_colour: u32,
    pub radius: f32,
    pub fill_segments: Vec<&'static str>,
    pub picker_body_visible: bool,
    pub sb_height: f32,
    pub track_height: f32,
    pub slider_thumb: f32,
    pub row_gap: f32,
    pub swatch_side: f32,
    pub previous_top_left: bool,
    pub previous_tooltip: &'static str,
    pub settings_symbol: &'static str,
    pub settings_tooltip: &'static str,
    pub menu: Vec<MenuItem>,
    pub notation: Notation,
    pub gamut_warning: Option<(&'static str, &'static str, u32, f32)>,
    pub gradient: bool,
    pub linear_angle: bool,
    pub midpoint_side: f32,
    pub recent: Vec<String>,
    pub focus_trapped: bool,
    pub ax_labels: [&'static str; 4],
}
#[derive(Clone, Debug)]
pub struct Picker {
    pub fill: Option<FillType>,
    pub space: Space,
    pub notation: Notation,
    pub hue_mapping: HueMapping,
    pub outside_project_gamut: bool,
    pub project_space: Space,
    pub previous: String,
    pub current: String,
    pub recent: Vec<String>,
}
impl Picker {
    pub fn plan(&self, tokens: &Tokens, glass: bool) -> PopoverPlan {
        let mut menu = Vec::new();
        for v in Space::ALL {
            menu.push(MenuItem {
                section: "Colour Space",
                label: v.label().into(),
                checked: v == self.space,
            });
        }
        for v in Notation::ALL {
            menu.push(MenuItem {
                section: "Notation",
                label: v.label().into(),
                checked: v == self.notation,
            });
        }
        menu.push(MenuItem {
            section: "",
            label: format!("Copy as {}", self.notation.label()),
            checked: false,
        });
        for v in HueMapping::ALL {
            menu.push(MenuItem {
                section: "Hue Mapping",
                label: v.label().into(),
                checked: v == self.hue_mapping,
            });
        }
        PopoverPlan {
            native_class: "NSPopover",
            width: tokens.metrics.popover_width,
            inset: tokens.metrics.inset,
            material: tokens.material(Surface::Popover, glass),
            content_colour: tokens.colors.floating,
            radius: tokens.metrics.control_radius,
            fill_segments: if self.fill.is_some() {
                FillType::ALL.map(FillType::label).to_vec()
            } else {
                vec![]
            },
            picker_body_visible: self.fill != Some(FillType::None),
            sb_height: tokens.metrics.popover_width / 2.0,
            track_height: tokens.metrics.keyframe + 3.0,
            slider_thumb: tokens.metrics.track_row / 2.0 + 2.0,
            row_gap: tokens.metrics.gap + 4.0,
            swatch_side: tokens.metrics.field,
            previous_top_left: true,
            previous_tooltip: RESTORE,
            settings_symbol: "ellipsis.circle",
            settings_tooltip: SETTINGS,
            menu,
            notation: self.notation,
            gamut_warning: (self.outside_project_gamut && self.project_space == Space::Srgb)
                .then_some((
                    "exclamationmark.triangle.fill",
                    GAMUT_WARNING,
                    tokens.colors.warning,
                    tokens.metrics.symbol_inline,
                )),
            gradient: matches!(self.fill, Some(FillType::Linear | FillType::Radial)),
            linear_angle: self.fill == Some(FillType::Linear),
            midpoint_side: tokens.metrics.keyframe - 2.0,
            recent: self.recent.iter().take(8).cloned().collect(),
            focus_trapped: true,
            ax_labels: [SB_LABEL, "Hue", "Opacity", EYEDROPPER],
        }
    }
    pub fn restore_previous(&mut self) {
        self.current.clone_from(&self.previous);
    }
    /// Called after an acknowledged commit, never for a preview or cancellation.
    pub fn remember(&mut self) {
        self.recent.retain(|c| c != &self.current);
        self.recent.insert(0, self.current.clone());
        self.recent.truncate(8);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RampKey {
    Left,
    Right,
    Home,
    End,
}
pub fn stop_key(percent: f64, key: RampKey, shift: bool) -> Option<f64> {
    if !percent.is_finite() {
        return None;
    }
    let step = if shift { 10.0 } else { 1.0 };
    Some(
        match key {
            RampKey::Left => percent - step,
            RampKey::Right => percent + step,
            RampKey::Home => 0.0,
            RampKey::End => 100.0,
        }
        .clamp(0.0, 100.0),
    )
}
pub fn midpoint_key(percent: f64, key: RampKey, shift: bool) -> Option<f64> {
    stop_key(percent, key, shift).map(|v| v.clamp(1.0, 99.0))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub id: String,
    pub percent: f64,
    pub colour: String,
}
/// Id-based selection survives reordered stops. Equal-position stops retain
/// their order; no deduplication or rewriting of authored stops.
#[derive(Clone, Debug, Default)]
pub struct Ramp {
    pub stops: Vec<Stop>,
    pub selected: Option<String>,
}
impl Ramp {
    /// The native gradient evaluator supplies the interpolated colour. Inserting
    /// a stop must not guess a different interpolation/colour space in the UI.
    pub fn add(&mut self, mut stop: Stop) -> bool {
        if !stop.percent.is_finite() || self.stops.iter().any(|s| s.id == stop.id) {
            return false;
        }
        stop.percent = stop.percent.clamp(0.0, 100.0);
        self.selected = Some(stop.id.clone());
        self.stops.push(stop);
        true
    }
    pub fn move_stop(&mut self, id: &str, percent: f64) -> bool {
        if !percent.is_finite() {
            return false;
        }
        let Some(stop) = self.stops.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        stop.percent = percent.clamp(0.0, 100.0);
        true
    }
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(index) = self.stops.iter().position(|s| s.id == id) else {
            return false;
        };
        self.stops.remove(index);
        if self.selected.as_deref() == Some(id) {
            self.selected = self
                .stops
                .get(index.min(self.stops.len().saturating_sub(1)))
                .map(|s| s.id.clone());
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismiss {
    Escape,
    OutsideClick,
    NotationReturn,
}
impl Dismiss {
    pub fn cancel(self) -> bool {
        self == Self::Escape
    }
    pub fn undo_label(property: &str) -> String {
        format!("Change {property}")
    }
}
