//! S021/S022/S029/S036/S043. Exact Opus copy and headless, ID-addressed AppKit
//! contracts. No document writer, JS, screen, file picker or invented visuals.
//!
//! Decisions (2026-11-01): N3 uses the authored specs, including Cancel as the
//! destructive default; N4 requires durable preset archival before removal;
//! N7 uses stable IDs and publication fences; N8 queues all edits/evaluation.
//! Sheet geometry resolves the approved host tokens without guessed dimensions.
//! All native output remains provisional until Opus review.

pub use crate::surfaces::sheet_layout;

#[cfg(target_os = "macos")]
#[path = "dialogs_appkit.rs"]
pub mod appkit;
#[path = "dialogs_kernel.rs"]
pub mod kernel;
#[cfg(test)]
#[path = "dialogs_tests.rs"]
mod tests;

use pm_builtins::RESOLUTIONS;
use std::collections::BTreeMap;

pub const NAME_ERROR: &str = "Enter a name.";
pub const SIZE_ERROR: &str = "Enter a size from 16 to 8192 pixels.";
pub const RATE_ERROR: &str = "Enter a frame rate from 1 to 240 fps.";
pub const DURATION_ERROR: &str = "Enter a duration up to 1 hour.";
pub const NUMBER_ERROR: &str = "Enter a number.";
pub const VELOCITY_ERROR: &str = "Enter a speed, and an influence from 0.1 % to 100 %.";
pub const COLLISION_ERROR: &str = "This would overlap another keyframe or go past the layer’s end.";
pub const TIME_PERCENT_ERROR: &str = "Enter a percentage greater than 0.";
pub const SCALE_TOOLTIP: &str = "Only number values can be scaled.";
pub const ADJUST_DURATION: &str =
    "Adjust composition duration to the time span of the selected layers";
pub const MOVE_ATTRIBUTES: &str = "Move all attributes into the new composition";
pub const MOVE_DESCRIPTION: &str = "Places the selected layers together into a new composition.";
pub const LEAVE_DESCRIPTION: &str = "Creates a composition with only this layer’s source in it. The new composition becomes the source of the layer.";
pub const LEAVE_DISABLED: &str = "Available for a single layer with a source.";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DialogId(pub String);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePiece {
    Sheet,
    TextField,
    NumberField,
    PopUpButton,
    Radio,
    Checkbox,
    ColourWell,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub piece: NativePiece,
    pub values: Vec<String>,
    /// The chosen entry of a pop-up's `values`.
    pub selected: usize,
    pub unit: String,
    pub placeholder: Option<&'static str>,
    pub enabled: bool,
    pub description: Option<String>,
    /// The description is a warning (drawn in the warning colour).
    pub warning: bool,
    pub tooltip: Option<&'static str>,
    pub error: Option<&'static str>,
    /// A secure text field (`NSSecureTextField`): what is typed never shows.
    pub secure: bool,
}
impl Field {
    pub fn new(id: &str, label: &str, piece: NativePiece, values: Vec<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            piece,
            values,
            selected: 0,
            unit: String::new(),
            placeholder: None,
            enabled: true,
            description: None,
            warning: false,
            tooltip: None,
            error: None,
            secure: false,
        }
    }
    pub fn number(id: &str, label: &str, values: Vec<String>, unit: &str) -> Self {
        Self {
            unit: unit.into(),
            ..Self::new(id, label, NativePiece::NumberField, values)
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub id: &'static str,
    pub label: &'static str,
    pub default: bool,
    pub destructive: bool,
    pub enabled: bool,
}
impl Button {
    pub fn action(label: &'static str) -> Self {
        Self {
            id: "apply",
            label,
            default: true,
            destructive: false,
            enabled: true,
        }
    }
    pub fn cancel(default: bool) -> Self {
        Self {
            id: "cancel",
            label: "Cancel",
            default,
            destructive: false,
            enabled: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct FieldGroup {
    pub title: &'static str,
    pub fields: Vec<&'static str>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub id: DialogId,
    pub owner: WindowId,
    /// An export alert is a sheet on the Export sheet, not on its editor.
    pub parent_sheet: Option<DialogId>,
    pub piece: NativePiece,
    pub title: String,
    pub line: Option<String>,
    pub fields: Vec<Field>,
    pub groups: Vec<FieldGroup>,
    /// Display order as specified. Default is explicit, independent of order.
    pub buttons: Vec<Button>,
    pub width_token: &'static str,
    pub inset_token: &'static str,
    pub value_column_token: &'static str,
    pub first_selected: Option<String>,
    pub expression_editor: bool,
    pub result_line: Option<String>,
    pub result_error: bool,
    /// A sheet-wide problem (a failed sign-in, a publish check) under the fields.
    pub warning: Option<String>,
}
impl Surface {
    pub fn sheet(
        id: DialogId,
        owner: WindowId,
        title: &str,
        fields: Vec<Field>,
        action: &'static str,
    ) -> Self {
        let first_selected = fields.first().map(|f| f.id.clone());
        Self {
            id,
            owner,
            parent_sheet: None,
            piece: NativePiece::Sheet,
            title: title.into(),
            line: None,
            fields,
            groups: vec![],
            buttons: vec![Button::cancel(false), Button::action(action)],
            width_token: "metrics.sheet_width",
            inset_token: "metrics.sheet_inset",
            value_column_token: "metrics.sheet_value_column",
            first_selected,
            expression_editor: false,
            result_line: None,
            result_error: false,
            warning: None,
        }
    }
    pub fn valid(&mut self) {
        let valid = self.fields.iter().all(|f| f.error.is_none());
        if let Some(b) = self.buttons.iter_mut().find(|b| b.id == "apply") {
            b.enabled = valid;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompositionDraft {
    pub name: String,
    pub width: f64,
    pub height: f64,
    pub fps: f64,
    pub duration: f64,
    pub background: String,
}
impl CompositionDraft {
    pub fn errors(&self) -> BTreeMap<&'static str, &'static str> {
        let mut e = BTreeMap::new();
        if self.name.trim().is_empty() {
            e.insert("name", NAME_ERROR);
        }
        if !in_range(self.width, 16., 8192.) || !in_range(self.height, 16., 8192.) {
            e.insert("size", SIZE_ERROR);
        }
        if !in_range(self.fps, 1., 240.) {
            e.insert("fps", RATE_ERROR);
        }
        if !in_range(self.duration, 0.1, 3600.) {
            e.insert("duration", DURATION_ERROR);
        }
        e
    }
    /// The sheet's fields as a draft. Text the person left as it opened keeps
    /// the exact value it shows (a duration of 2206 frames at 30 fps reads
    /// `73.533` but stays frame-exact); unparseable numbers stay invalid.
    pub fn read(&self, text: impl Fn(&str) -> String, background: Option<String>) -> Self {
        let number = |id: &str, original: f64| {
            let typed = text(id);
            if typed.trim() == number_text(original) {
                original
            } else {
                typed.trim().parse().unwrap_or(f64::NAN)
            }
        };
        Self {
            name: text("name"),
            width: number("size.0", self.width),
            height: number("size.1", self.height),
            fps: number("fps", self.fps),
            duration: number("duration", self.duration),
            background: background.unwrap_or_else(|| self.background.clone()),
        }
    }
    /// The Duration row's detail: the length in frames, as v1's settings say
    /// it, or the layers a shorter duration cuts off (`cut`).
    pub fn duration_detail(&self, cut: usize) -> (String, bool) {
        match cut {
            0 if in_range(self.duration, 0.1, 3600.) && in_range(self.fps, 1., 240.) => (
                format!(
                    "Total length of the composition — {} frames at {} fps.",
                    (self.duration * self.fps).round().max(1.),
                    number_text(self.fps)
                ),
                false,
            ),
            0 => ("Total length of the composition, in seconds.".into(), false),
            1 => (CUT_ONE.into(), true),
            n => (cut_many(n), true),
        }
    }
    pub fn surface(&self, id: DialogId, owner: WindowId, settings: bool, cut: usize) -> Surface {
        let (detail, warning) = self.duration_detail(cut);
        let mut fields = vec![
            Field::new(
                "name",
                "Name",
                NativePiece::TextField,
                vec![self.name.clone()],
            ),
            Field {
                selected: self.resolution_index(),
                ..Field::new(
                    "resolution",
                    "Resolution",
                    NativePiece::PopUpButton,
                    resolution_labels(),
                )
            },
            Field::number(
                "size",
                "Size",
                vec![number_text(self.width), number_text(self.height)],
                "px",
            ),
            Field::number("fps", "Frame Rate", vec![number_text(self.fps)], "fps"),
            Field {
                description: Some(detail),
                warning,
                ..Field::number(
                    "duration",
                    "Duration",
                    vec![number_text(self.duration)],
                    "s",
                )
            },
            Field::new(
                "background",
                "Background",
                NativePiece::ColourWell,
                vec![self.background.clone()],
            ),
        ];
        let errors = self.errors();
        for f in &mut fields {
            f.error = errors.get(f.id.as_str()).copied();
        }
        let mut s = Surface::sheet(
            id,
            owner,
            if settings {
                "Composition Settings"
            } else {
                "New Composition"
            },
            fields,
            if settings { "Apply" } else { "Create" },
        );
        s.valid();
        s
    }
    /// The resolution pop-up's entry for this size. `Custom` is the last.
    pub fn resolution_index(&self) -> usize {
        RESOLUTIONS
            .iter()
            .position(|(_, w, h)| f64::from(*w) == self.width && f64::from(*h) == self.height)
            .unwrap_or(RESOLUTIONS.len())
    }
    pub fn choose_resolution(&mut self, label: &str) -> bool {
        if let Some((_, w, h)) = RESOLUTIONS.iter().find(|r| r.0 == label) {
            self.width = f64::from(*w);
            self.height = f64::from(*h);
            true
        } else {
            false
        }
    }
}
fn resolution_labels() -> Vec<String> {
    RESOLUTIONS
        .iter()
        .map(|r| r.0.to_owned())
        .chain(["Custom".to_owned()])
        .collect()
}
/// A number as a sheet field shows it: at most three decimals, no trailing zeros.
pub fn number_text(n: f64) -> String {
    if !n.is_finite() {
        return n.to_string();
    }
    let text = format!("{n:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}
pub const CUT_ONE: &str = "1 layer runs past this duration and stops at the end.";
pub fn cut_many(n: usize) -> String {
    format!("{n} layers run past this duration and stop at the end.")
}
fn in_range(n: f64, min: f64, max: f64) -> bool {
    n.is_finite() && (min..=max).contains(&n)
}
pub fn unique_name(prefix: &str, names: &[String]) -> String {
    (1..)
        .map(|n| format!("{prefix} {n}"))
        .find(|n| !names.contains(n))
        .expect("finite names")
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrecomposeDraft {
    pub name: String,
    pub layer_name: Option<String>,
    pub can_leave: bool,
    pub leave: bool,
    pub adjust_duration: bool,
    pub open_after: bool,
}
impl PrecomposeDraft {
    pub fn surface(&self, id: DialogId, owner: WindowId) -> Surface {
        let mut leave = Field::new(
            "leave",
            &self
                .layer_name
                .as_ref()
                .filter(|_| self.can_leave)
                .map(|n| format!("Leave all attributes in “{n}”"))
                .unwrap_or_else(|| "Leave all attributes in the layer".into()),
            NativePiece::Radio,
            vec![(self.leave && self.can_leave).to_string()],
        );
        leave.enabled = self.can_leave;
        leave.description = Some(
            if self.can_leave {
                LEAVE_DESCRIPTION
            } else {
                LEAVE_DISABLED
            }
            .into(),
        );
        let mut move_all = Field::new(
            "move",
            MOVE_ATTRIBUTES,
            NativePiece::Radio,
            vec![(!self.leave || !self.can_leave).to_string()],
        );
        move_all.description = Some(MOVE_DESCRIPTION.into());
        let mut adjust = Field::new(
            "adjust",
            ADJUST_DURATION,
            NativePiece::Checkbox,
            vec![self.adjust_duration.to_string()],
        );
        adjust.enabled = !self.leave || !self.can_leave;
        Surface::sheet(
            id,
            owner,
            "Pre-compose",
            vec![
                Field::new(
                    "name",
                    "Name",
                    NativePiece::TextField,
                    vec![self.name.clone()],
                ),
                leave,
                move_all,
                adjust,
                Field::new(
                    "open",
                    "Open New Composition",
                    NativePiece::Checkbox,
                    vec![self.open_after.to_string()],
                ),
            ],
            "Pre-compose",
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixedNumber {
    pub value: Option<f64>,
    pub draft: String,
}
impl MixedNumber {
    /// What a person typed. Blank keeps each key's own value.
    pub fn parsed_input(text: &str) -> Self {
        Self {
            draft: text.into(),
            value: text.trim().parse().ok(),
        }
    }
    pub fn new(values: &[f64]) -> Self {
        let value = values
            .first()
            .copied()
            .filter(|v| values.iter().all(|n| n == v));
        Self {
            value,
            draft: value.map_or(String::new(), |v| v.to_string()),
        }
    }
    pub fn parsed(&self) -> Result<Option<f64>, &'static str> {
        if self.draft.trim().is_empty() {
            return if self.value.is_none() {
                Ok(None)
            } else {
                Err(NUMBER_ERROR)
            };
        }
        self.draft
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or(NUMBER_ERROR)
    }
    pub fn field(&self, id: &str, label: &str, unit: &str) -> Field {
        let mut f = Field::number(id, label, vec![self.draft.clone()], unit);
        if self.value.is_none() {
            f.placeholder = Some("Mixed");
        }
        f.error = self.parsed().err();
        f
    }
}
pub fn keyframe_value(
    id: DialogId,
    owner: WindowId,
    property: &str,
    timecode: &str,
    count: usize,
    components: &[MixedNumber],
    unit: &str,
) -> Surface {
    let mut f = Field::number(
        "value",
        property,
        components.iter().map(|c| c.draft.clone()).collect(),
        unit,
    );
    if components.iter().any(|c| c.value.is_none()) {
        f.placeholder = Some("Mixed");
    }
    f.error = components.iter().find_map(|c| c.parsed().err());
    let mut s = Surface::sheet(id, owner, "Keyframe Value", vec![f], "Apply");
    s.line = Some(if count == 1 {
        format!("{property} at {timecode}")
    } else {
        format!("{property} · {count} keyframes")
    });
    s.valid();
    s
}
#[derive(Clone, Debug, PartialEq)]
pub struct VelocityDraft {
    pub incoming_speed: MixedNumber,
    pub incoming_influence: MixedNumber,
    pub outgoing_speed: MixedNumber,
    pub outgoing_influence: MixedNumber,
}
impl VelocityDraft {
    pub fn valid(&self) -> bool {
        [&self.incoming_speed, &self.outgoing_speed]
            .iter()
            .all(|n| n.parsed().is_ok())
            && [&self.incoming_influence, &self.outgoing_influence]
                .iter()
                .all(|n| {
                    n.parsed()
                        .is_ok_and(|v| v.is_none_or(|v| in_range(v, 0.1, 100.)))
                })
    }
    pub fn surface(&self, id: DialogId, owner: WindowId, count: usize, unit: &str) -> Surface {
        let mut s = Surface::sheet(
            id,
            owner,
            "Keyframe Velocity",
            vec![
                self.incoming_speed
                    .field("in.speed", "Speed", &format!("{unit}/s")),
                self.incoming_influence
                    .field("in.influence", "Influence", "%"),
                self.outgoing_speed
                    .field("out.speed", "Speed", &format!("{unit}/s")),
                self.outgoing_influence
                    .field("out.influence", "Influence", "%"),
            ],
            "Apply",
        );
        s.groups = vec![
            FieldGroup {
                title: "Incoming",
                fields: vec!["in.speed", "in.influence"],
            },
            FieldGroup {
                title: "Outgoing",
                fields: vec!["out.speed", "out.influence"],
            },
        ];
        s.line = Some(if count == 1 {
            "1 keyframe".into()
        } else {
            format!("{count} keyframes")
        });
        if !self.valid() {
            s.fields[0].error = Some(VELOCITY_ERROR);
        }
        s.valid();
        s
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleDraft {
    pub time_percent: f64,
    pub value_percent: f64,
    pub numeric: bool,
    pub collision: bool,
}
impl Default for ScaleDraft {
    fn default() -> Self {
        Self {
            time_percent: 100.,
            value_percent: 100.,
            numeric: true,
            collision: false,
        }
    }
}
impl ScaleDraft {
    pub fn valid(&self) -> bool {
        self.time_percent.is_finite()
            && self.time_percent > 0.
            && (!self.numeric || self.value_percent.is_finite())
            && !self.collision
    }
    pub fn surface(&self, id: DialogId, owner: WindowId) -> Surface {
        let mut time = Field::number("time", "Time", vec![self.time_percent.to_string()], "%");
        let mut value = Field::number("value", "Value", vec![self.value_percent.to_string()], "%");
        time.error = (!self.time_percent.is_finite() || self.time_percent <= 0.)
            .then_some(TIME_PERCENT_ERROR);
        value.error = (self.numeric && !self.value_percent.is_finite()).then_some(NUMBER_ERROR);
        value.enabled = self.numeric;
        if !self.numeric {
            value.tooltip = Some(SCALE_TOOLTIP);
        }
        let mut s = Surface::sheet(id, owner, "Scale Keyframes", vec![time, value], "Apply");
        s.line = Some("Scales from the earliest selected keyframe.".into());
        if self.collision {
            s.fields[0].error = Some(COLLISION_ERROR);
        }
        s.buttons[1].enabled = self.valid();
        s
    }
}
