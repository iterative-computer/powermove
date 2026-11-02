//! S051: system menus and input services, without an app context-menu replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Number,
    Timecode,
    Notation,
    Expression,
    Code,
    Canvas,
    AgentComposer,
    LayerName,
    Notes,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Setting {
    #[default]
    SystemDefault,
    Off,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextPolicy {
    pub spelling: Setting,
    pub grammar: Setting,
    pub substitutions: Setting,
    pub smart_quotes: Setting,
    pub writing_tools: Setting,
}
impl FieldKind {
    pub fn policy(self) -> TextPolicy {
        use Setting::Off;
        match self {
            Self::Number | Self::Timecode | Self::Notation => TextPolicy {
                spelling: Off,
                grammar: Off,
                substitutions: Off,
                writing_tools: Off,
                smart_quotes: Off,
            },
            Self::Expression | Self::Code => TextPolicy {
                spelling: Off,
                substitutions: Off,
                smart_quotes: Off,
                ..TextPolicy::default()
            },
            _ => TextPolicy::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Focus {
    previous_panel: Option<String>,
    field: Option<String>,
    marked_text: bool,
}
impl Focus {
    pub fn enter(&mut self, field: &str, previous_panel: &str) {
        // Moving between fields retains the panel that originally had key focus.
        if self.field.is_none() {
            self.previous_panel = Some(previous_panel.into());
        }
        self.field = Some(field.into());
    }
    pub fn set_marked_text(&mut self, active: bool) {
        self.marked_text = active;
    }
    pub fn editor_shortcuts_allowed(&self) -> bool {
        self.field.is_none()
    }
    pub fn escape(&mut self) -> Option<String> {
        // The input manager consumes Esc while it owns marked text.
        if self.marked_text {
            return None;
        }
        self.field.take();
        self.previous_panel.take()
    }
    pub fn native_resigned(&mut self) {
        self.field = None;
        self.marked_text = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEntry {
    DoubleClick,
    TextToolClick,
    SelectedLayerReturn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextExit {
    Escape,
    CommandReturn,
    OutsideClick,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextCreation {
    Point,
    Paragraph { width: f64 },
}
impl TextCreation {
    /// The viewer's existing gesture threshold decides whether this was a drag.
    /// Tagged paragraph creation fixes width and leaves height automatic.
    pub fn from_gesture(dragged_width: Option<f64>) -> Option<Self> {
        match dragged_width {
            None => Some(Self::Point),
            Some(w) if w.is_finite() && w > 0.0 => Some(Self::Paragraph { width: w }),
            _ => None,
        }
    }
}

/// IME draft is separate from committed authored text. All exit routes commit;
/// native input must finish marked text first. No custom caret or IME underline.
#[derive(Clone, Debug)]
pub struct CanvasText {
    pub layer: String,
    pub draft: String,
    marked: bool,
    finished: bool,
}
impl CanvasText {
    pub fn new(layer: &str, text: &str) -> Self {
        Self {
            layer: layer.into(),
            draft: text.into(),
            marked: false,
            finished: false,
        }
    }
    pub fn composing(&mut self, active: bool) {
        self.marked = active;
    }
    pub fn replace(&mut self, text: String) -> bool {
        if self.finished {
            return false;
        }
        self.draft = text;
        true
    }
    pub fn finish(&mut self, _reason: TextExit) -> Option<&str> {
        if self.finished || self.marked {
            return None;
        }
        self.finished = true;
        Some(&self.draft)
    }
}
