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

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// Review answers implemented on 2026-11-01 (N3: authored intent).
pub const DESIGN_TODOS: &[&str] = &[];

pub const NAME_ERROR: &str = "Enter a name.";
pub const SIZE_ERROR: &str = "Enter a size from 16 to 8192 pixels.";
pub const RATE_ERROR: &str = "Enter a frame rate from 1 to 240 fps.";
pub const DURATION_ERROR: &str = "Enter a duration up to 1 hour.";
pub const NUMBER_ERROR: &str = "Enter a number.";
pub const VELOCITY_ERROR: &str = "Enter a speed, and an influence from 0.1 % to 100 %.";
pub const COLLISION_ERROR: &str = "This would overlap another keyframe or go past the layer’s end.";
pub const TIME_PERCENT_ERROR: &str = "Enter a percentage greater than 0.";
pub const PRESET_NAME_ERROR: &str = "A preset with that name already exists.";
pub const SCALE_TOOLTIP: &str = "Only number values can be scaled.";
pub const ADJUST_DURATION: &str =
    "Adjust composition duration to the time span of the selected layers";
pub const MOVE_ATTRIBUTES: &str = "Move all attributes into the new composition";
pub const MOVE_DESCRIPTION: &str = "Places the selected layers together into a new composition.";
pub const LEAVE_DESCRIPTION: &str = "Creates a composition with only this layer’s source in it. The new composition becomes the source of the layer.";
pub const LEAVE_DISABLED: &str = "Available for a single layer with a source.";
pub const PRESET_FAILURE: &str = "Couldn’t save the preset.";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DialogId(pub String);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePiece {
    Sheet,
    Alert,
    TextField,
    NumberField,
    PopUpButton,
    Radio,
    Checkbox,
    ColourWell,
    TextView,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub piece: NativePiece,
    pub values: Vec<String>,
    pub unit: String,
    pub placeholder: Option<&'static str>,
    pub enabled: bool,
    pub description: Option<String>,
    pub tooltip: Option<&'static str>,
    pub error: Option<&'static str>,
}
impl Field {
    pub fn new(id: &str, label: &str, piece: NativePiece, values: Vec<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            piece,
            values,
            unit: String::new(),
            placeholder: None,
            enabled: true,
            description: None,
            tooltip: None,
            error: None,
        }
    }
    pub fn number(id: &str, label: &str, values: Vec<String>, unit: &str) -> Self {
        Self {
            unit: unit.into(),
            ..Self::new(id, label, NativePiece::NumberField, values)
        }
    }
    /// NSTextField labels / NSAccessibility title, never an internal ID.
    pub fn voiceover_label(&self) -> &str {
        &self.label
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
    pub warning: Option<String>,
    pub width_token: &'static str,
    pub inset_token: &'static str,
    pub value_column_token: &'static str,
    pub first_selected: Option<String>,
    pub expression_editor: bool,
    pub result_line: Option<String>,
    pub result_error: bool,
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
            warning: None,
            width_token: "metrics.sheet_width",
            inset_token: "metrics.sheet_inset",
            value_column_token: "metrics.sheet_value_column",
            first_selected,
            expression_editor: false,
            result_line: None,
            result_error: false,
        }
    }
    pub fn valid(&mut self) {
        let valid = self.fields.iter().all(|f| f.error.is_none());
        if let Some(b) = self.buttons.iter_mut().find(|b| b.id == "apply") {
            b.enabled = valid;
        }
    }
    pub fn keyboard(&self, key: Key) -> KeyEffect {
        match key {
            Key::Escape => KeyEffect::Cancel,
            Key::CommandReturn if self.expression_editor => self.apply_key(),
            Key::Return | Key::Tab if self.expression_editor => KeyEffect::Type,
            Key::ControlTab if self.expression_editor => KeyEffect::LeaveEditor,
            Key::Return => self.apply_key(),
            Key::Tab => KeyEffect::NextInReadingOrder,
            _ => KeyEffect::Unhandled,
        }
    }
    fn apply_key(&self) -> KeyEffect {
        self.buttons
            .iter()
            .find(|b| b.default && b.enabled)
            .map_or(KeyEffect::Unhandled, |b| {
                if b.id == "cancel" {
                    KeyEffect::Cancel
                } else {
                    KeyEffect::Apply
                }
            })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    CommandReturn,
    Escape,
    Tab,
    ControlTab,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEffect {
    Apply,
    Cancel,
    Type,
    LeaveEditor,
    NextInReadingOrder,
    Unhandled,
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
    pub fn surface(
        &self,
        id: DialogId,
        owner: WindowId,
        settings: bool,
        extending_layers: usize,
    ) -> Surface {
        let mut fields = vec![
            Field::new(
                "name",
                "Name",
                NativePiece::TextField,
                vec![self.name.clone()],
            ),
            Field::new(
                "resolution",
                "Resolution",
                NativePiece::PopUpButton,
                vec![self.resolution().into()],
            ),
            Field::number(
                "size",
                "Size",
                vec![self.width.to_string(), self.height.to_string()],
                "px",
            ),
            Field::number("fps", "Frame Rate", vec![self.fps.to_string()], "fps"),
            Field::number("duration", "Duration", vec![self.duration.to_string()], "s"),
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
        if settings && extending_layers > 0 {
            s.warning = Some(if extending_layers == 1 {
                "1 layer extends past the new duration. It’s kept but won’t play past the end."
                    .into()
            } else {
                format!(
                    "{extending_layers} layers extend past the new duration. They’re kept but won’t play past the end."
                )
            });
        }
        s.valid();
        s
    }
    pub fn resolution(&self) -> &'static str {
        RESOLUTIONS
            .iter()
            .find(|(_, w, h)| *w == self.width && *h == self.height)
            .map_or("Custom", |r| r.0)
    }
    pub fn choose_resolution(&mut self, label: &str) -> bool {
        if let Some((_, w, h)) = RESOLUTIONS.iter().find(|r| r.0 == label) {
            self.width = *w;
            self.height = *h;
            true
        } else {
            false
        }
    }
}
pub const RESOLUTIONS: &[(&str, f64, f64)] = &[
    ("4K UHD · 3840×2160", 3840., 2160.),
    ("HD 1080p · 1920×1080", 1920., 1080.),
    ("HD 720p · 1280×720", 1280., 720.),
    ("Vertical HD · 1080×1920", 1080., 1920.),
    ("Portrait 4:5 · 1080×1350", 1080., 1350.),
    ("Square · 1080×1080", 1080., 1080.),
];
fn in_range(n: f64, min: f64, max: f64) -> bool {
    n.is_finite() && (min..=max).contains(&n)
}
pub fn unique_name(prefix: &str, names: &[String]) -> String {
    (1..)
        .map(|n| format!("{prefix} {n}"))
        .find(|n| !names.contains(n))
        .expect("finite names")
}

/// Timecode entry uses the current composition's integer displayed frame rate;
/// storage remains seconds. NumberField owns arithmetic and relative stepping.
pub fn duration_input(input: &str, fps: f64) -> Option<f64> {
    if !input.contains(':') {
        return input.trim().parse::<f64>().ok().filter(|n| n.is_finite());
    }
    if !fps.is_finite() || fps <= 0. {
        return None;
    }
    let parts: Vec<_> = input.trim().split(':').collect();
    if parts.len() != 4 {
        return None;
    }
    let p: Vec<u64> = parts
        .iter()
        .map(|s| s.parse())
        .collect::<Result<_, _>>()
        .ok()?;
    if p[1] >= 60 || p[2] >= 60 || p[3] as f64 >= fps.round() {
        return None;
    }
    Some(p[0] as f64 * 3600. + p[1] as f64 * 60. + p[2] as f64 + p[3] as f64 / fps)
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

pub fn delete_composition(id: DialogId, owner: WindowId, name: &str, users: usize) -> Surface {
    alert(
        id,
        owner,
        format!("Delete “{name}”?"),
        Some(if users == 0 {
            "You can undo this.".into()
        } else if users == 1 {
            "It’s used in 1 other composition. Those layers are removed too. You can undo this."
                .into()
        } else {
            format!(
                "It’s used in {users} other compositions. Those layers are removed too. You can undo this."
            )
        }),
        "Delete",
        true,
    )
}
fn alert(
    id: DialogId,
    owner: WindowId,
    title: String,
    line: Option<String>,
    action: &'static str,
    destructive: bool,
) -> Surface {
    let mut s = Surface::sheet(id, owner, &title, vec![], action);
    s.piece = NativePiece::Alert;
    s.line = line;
    // NSAlert order: the first button added is rightmost and the default, so
    // Cancel (default, Esc) goes first and the action sits to its left.
    s.buttons = vec![
        Button::cancel(true),
        Button {
            default: false,
            destructive,
            ..Button::action(action)
        },
    ];
    s
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
pub const KEYFRAME_MENU: &[&str] = &["Edit Value…", "Keyframe Velocity…", "Scale Keyframes…"];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyframeMenuItem {
    pub id: &'static str,
    pub label: &'static str,
    pub enabled: bool,
}
pub fn keyframe_menu(editable_keys: usize) -> Vec<KeyframeMenuItem> {
    ["edit-value", "keyframe-velocity", "scale-keyframes"]
        .into_iter()
        .zip(KEYFRAME_MENU.iter().copied())
        .map(|(id, label)| KeyframeMenuItem {
            id,
            label,
            enabled: editable_keys > 0,
        })
        .collect()
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
pub const VELOCITY_GROUPS: &[&str] = &["Incoming", "Outgoing"];
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Pending,
    Value(String),
    Syntax { line: usize, message: String },
    Runtime { line: usize, message: String },
    SyntaxUnlocated { message: String },
    RuntimeUnlocated { message: String },
    Unavailable,
}
impl Preview {
    pub fn line(&self) -> Option<String> {
        match self {
            Self::Value(v) => Some(format!("At the playhead: {v}")),
            Self::Syntax { line, message } | Self::Runtime { line, message } => {
                Some(format!("Line {line}: {message}"))
            }
            Self::SyntaxUnlocated { message } | Self::RuntimeUnlocated { message } => {
                Some(message.clone())
            }
            Self::Unavailable => Some("No value at the playhead.".into()),
            Self::Pending => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ExpressionDraft {
    pub source: String,
    pub existed: bool,
    pub layer: String,
    pub property: String,
    pub preview: Preview,
    previous_result: Option<String>,
    previous_result_error: bool,
    generation: u64,
    due: Option<Instant>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluationJob {
    pub generation: u64,
    pub source: String,
}
impl ExpressionDraft {
    pub fn new(source: String, layer: String, property: String) -> Self {
        Self {
            existed: !source.is_empty(),
            source,
            layer,
            property,
            preview: Preview::Pending,
            previous_result: None,
            previous_result_error: false,
            generation: 0,
            due: Some(Instant::now() + Duration::from_millis(250)),
        }
    }
    pub fn edit(&mut self, source: String, now: Instant) {
        self.source = source;
        self.generation += 1;
        if !matches!(self.preview, Preview::Pending) {
            self.previous_result = self.preview.line();
            self.previous_result_error = matches!(
                self.preview,
                Preview::Syntax { .. }
                    | Preview::Runtime { .. }
                    | Preview::SyntaxUnlocated { .. }
                    | Preview::RuntimeUnlocated { .. }
            );
        }
        self.preview = Preview::Pending;
        self.due = Some(now + Duration::from_millis(250));
    }
    /// Poll on a worker; the caller must enqueue evaluation, never run JS on main.
    pub fn take_job(&mut self, now: Instant) -> Option<EvaluationJob> {
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            Some(EvaluationJob {
                generation: self.generation,
                source: self.source.clone(),
            })
        } else {
            None
        }
    }
    /// The host posts an NSAccessibility .medium announcement only on true.
    pub fn complete(&mut self, job: &EvaluationJob, preview: Preview) -> bool {
        if job.generation != self.generation || job.source != self.source {
            return false;
        }
        let changed = self.preview != preview;
        self.preview = preview;
        changed
    }
    pub fn complete_with_host(
        &mut self,
        job: &EvaluationJob,
        preview: Preview,
        owner: WindowId,
        host: &mut impl DialogHost,
    ) -> bool {
        let previous = if matches!(self.preview, Preview::Pending) {
            self.previous_result.clone()
        } else {
            self.preview.line()
        };
        if !self.complete(job, preview) {
            return false;
        }
        if let Some(text) = self
            .preview
            .line()
            .filter(|text| Some(text) != previous.as_ref())
        {
            host.enqueue(HostEvent::Announce {
                owner,
                text,
                priority: Announcement::Medium,
            });
        }
        true
    }
    /// Cocoa selections use UTF-16 offsets. Invalid/split-surrogate ranges refuse.
    pub fn insert(&mut self, range: std::ops::Range<usize>, text: &str, now: Instant) -> bool {
        fn byte(s: &str, offset: usize) -> Option<usize> {
            let mut n = 0;
            for (i, c) in s.char_indices() {
                if n == offset {
                    return Some(i);
                }
                n += c.len_utf16();
            }
            (n == offset).then_some(s.len())
        }
        let (Some(start), Some(end)) = (
            byte(&self.source, range.start),
            byte(&self.source, range.end),
        ) else {
            return false;
        };
        if start > end {
            return false;
        }
        let mut source = self.source.clone();
        source.replace_range(start..end, text);
        self.edit(source, now);
        true
    }
    pub fn surface(&self, id: DialogId, owner: WindowId) -> Surface {
        let f = Field::new(
            "expression",
            &format!("Expression for {}", self.property),
            NativePiece::TextView,
            vec![self.source.clone()],
        );
        let mut s = Surface::sheet(id, owner, "Expression", vec![f], "Apply");
        s.width_token = "metrics.sheet_width_wide";
        s.line = Some(format!("{} · {}", self.layer, self.property));
        s.expression_editor = true;
        s.result_line = if matches!(self.preview, Preview::Pending) {
            self.previous_result.clone()
        } else {
            self.preview.line()
        };
        s.result_error = (matches!(self.preview, Preview::Pending) && self.previous_result_error)
            || matches!(
                self.preview,
                Preview::Syntax { .. }
                    | Preview::Runtime { .. }
                    | Preview::SyntaxUnlocated { .. }
                    | Preview::RuntimeUnlocated { .. }
            );
        s.buttons[1].enabled = !matches!(
            self.preview,
            Preview::Syntax { .. }
                | Preview::SyntaxUnlocated { .. }
                | Preview::Pending
                | Preview::Unavailable
        );
        if self.existed {
            s.buttons.insert(
                0,
                Button {
                    id: "remove",
                    label: "Remove Expression",
                    default: false,
                    destructive: true,
                    enabled: true,
                },
            );
        }
        s
    }
}
/// AppKit NSTextView configuration: P-TEXT, ten scrollable lines, MONO_FONT / t.body.
pub const EXPRESSION_TEXT: ExpressionText = ExpressionText {
    lines: 10,
    smart_quotes: false,
    smart_dashes: false,
    substitutions: false,
    spell_checking: false,
    line_numbers: false,
    syntax_colouring: false,
    font: pm_design::MONO_FONT,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpressionText {
    pub lines: usize,
    pub smart_quotes: bool,
    pub smart_dashes: bool,
    pub substitutions: bool,
    pub spell_checking: bool,
    pub line_numbers: bool,
    pub syntax_colouring: bool,
    pub font: &'static str,
}
pub const INSERT_TITLE: &str = "Insert ⌄";

pub fn replace_preset(id: DialogId, owner: WindowId, export: DialogId, name: &str) -> Surface {
    let mut s = alert(
        id,
        owner,
        format!("A preset named “{name}” already exists. Do you want to replace it?"),
        None,
        "Replace",
        false,
    );
    s.parent_sheet = Some(export);
    s
}
pub fn rename_preset(id: DialogId, owner: WindowId, export: DialogId, name: &str) -> Surface {
    let mut s = alert(id, owner, "Rename Preset".into(), None, "Rename", false);
    s.parent_sheet = Some(export);
    // Rename is the default here: it goes first (rightmost), Cancel after it.
    s.buttons.reverse();
    s.buttons[0].default = true;
    s.buttons[1].default = false;
    s.fields.push(Field::new(
        "name",
        "Name",
        NativePiece::TextField,
        vec![name.into()],
    ));
    s.first_selected = Some("name".into());
    s
}
/// S043 review answer: validate without mutating or submitting an unchanged name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameValidation {
    Empty,
    Unchanged,
    Duplicate,
    Ready(String),
}
impl RenameValidation {
    pub fn check(original: &str, draft: &str, other_names: &[String]) -> Self {
        let name = draft.trim();
        if name.is_empty() {
            Self::Empty
        } else if name == original {
            Self::Unchanged
        } else if other_names.iter().any(|other| other == name) {
            Self::Duplicate
        } else {
            Self::Ready(name.into())
        }
    }
    pub fn apply(&self, surface: &mut Surface) {
        surface.fields[0].error = matches!(self, Self::Duplicate).then_some(PRESET_NAME_ERROR);
        surface.buttons[0].enabled = !matches!(self, Self::Empty | Self::Duplicate);
        surface.buttons[0].id = if matches!(self, Self::Unchanged) {
            "cancel"
        } else {
            "apply"
        };
    }
    pub fn key_effect(&self) -> KeyEffect {
        match self {
            Self::Unchanged => KeyEffect::Cancel,
            Self::Ready(_) => KeyEffect::Apply,
            _ => KeyEffect::Unhandled,
        }
    }
}
pub fn delete_preset(id: DialogId, owner: WindowId, export: DialogId, name: &str) -> Surface {
    let mut s = alert(
        id,
        owner,
        format!("Delete the preset “{name}”?"),
        None,
        "Delete",
        true,
    );
    s.parent_sheet = Some(export);
    s
}
pub fn preset_menu(current: Option<&str>) -> Vec<String> {
    current.map_or_else(Vec::new, |n| {
        vec![
            String::new(),
            format!("Rename “{n}”…"),
            format!("Delete “{n}”"),
        ]
    })
}
/// Accessory width 240 is explicit in S043.
pub const PRESET_RENAME_ACCESSORY_WIDTH: f64 = 240.;

/// Nonblocking host seam: AppKit uses NSAlert.beginSheetModal / NSWindow.beginSheet;
/// system placement and app icon; no custom alert semantics/critical badge.
/// Main-thread adapters enqueue ID-addressed input; they never evaluate JS.
pub trait DialogHost {
    fn enqueue(&mut self, event: HostEvent);
}
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    Present(Surface),
    Update(Surface),
    Close {
        id: DialogId,
        restore_trigger: bool,
    },
    Banner {
        owner: WindowId,
        text: String,
        announcement: Announcement,
    },
    Announce {
        owner: WindowId,
        text: String,
        priority: Announcement,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announcement {
    Medium,
    High,
}
#[derive(Default)]
pub struct HeadlessHost {
    pub events: Vec<HostEvent>,
}
impl DialogHost for HeadlessHost {
    fn enqueue(&mut self, event: HostEvent) {
        self.events.push(event);
    }
}

/// One alert per owning window, FIFO. Different document windows are independent.
#[derive(Default)]
pub struct DialogQueue {
    windows: BTreeMap<WindowId, VecDeque<Surface>>,
}
impl DialogQueue {
    pub fn present(&mut self, surface: Surface, host: &mut impl DialogHost) -> bool {
        if self.windows.values().flatten().any(|s| s.id == surface.id) {
            return false;
        }
        let q = self.windows.entry(surface.owner.clone()).or_default();
        if q.is_empty() {
            host.enqueue(HostEvent::Present(surface.clone()));
        }
        q.push_back(surface);
        true
    }
    pub fn close(&mut self, id: &DialogId, host: &mut impl DialogHost) -> bool {
        let Some(q) = self
            .windows
            .values_mut()
            .find(|q| q.iter().any(|s| &s.id == id))
        else {
            return false;
        };
        let i = q.iter().position(|s| &s.id == id).expect("found");
        q.remove(i);
        if i == 0 {
            host.enqueue(HostEvent::Close {
                id: id.clone(),
                restore_trigger: q.is_empty(),
            });
            if let Some(s) = q.front() {
                host.enqueue(HostEvent::Present(s.clone()));
            }
        }
        true
    }
    pub fn stale(&mut self, id: &DialogId, object: &str, host: &mut impl DialogHost) -> bool {
        let owner = self
            .windows
            .values()
            .flatten()
            .find(|s| &s.id == id)
            .map(|s| s.owner.clone());
        if !self.close(id, host) {
            return false;
        }
        host.enqueue(HostEvent::Banner {
            owner: owner.expect("found"),
            text: format!("{object} was changed, so the dialog closed."),
            announcement: Announcement::Medium,
        });
        true
    }
}

/// Platform presentation boundary shared by the real AppKit alerts and Linux
/// double. Sheet forms use the resolved shared sheet_layout geometry.
pub trait AlertHost {
    type Error;
    fn show_alert(&mut self, surface: &Surface) -> Result<(), Self::Error>;
}
impl AlertHost for HeadlessHost {
    type Error = &'static str;
    fn show_alert(&mut self, surface: &Surface) -> Result<(), Self::Error> {
        if surface.piece != NativePiece::Alert {
            return Err("not an alert");
        }
        self.enqueue(HostEvent::Present(surface.clone()));
        Ok(())
    }
}
