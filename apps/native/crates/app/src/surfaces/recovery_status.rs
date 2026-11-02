//! Status uses pm-design token references; layout/painting belongs to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionItem {
    pub text: String,
    pub tooltip: Option<String>,
}
impl ExtensionItem {
    pub fn new(text: &str) -> Self {
        let truncated = text.chars().count() > 24;
        Self {
            text: if truncated {
                format!("{}…", text.chars().take(23).collect::<String>())
            } else {
                text.into()
            },
            tooltip: truncated.then(|| text.into()),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub layers: u64,
    pub keyframes: u64,
    pub playing: bool,
    pub width: u32,
    pub height: u32,
    /// Caller formats the exact authored rational rate without a lossy integer cast.
    pub rate: String,
    pub workspace: String,
    pub extension_items: Vec<ExtensionItem>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusText {
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub indicator_label: Option<String>,
}
impl Status {
    pub const AX_LABEL: &'static str = "Status";
    pub fn text(&self, issue_count: usize) -> StatusText {
        let mut left = vec![
            format!(
                "{} {}",
                self.layers,
                if self.layers == 1 { "layer" } else { "layers" }
            ),
            format!(
                "{} {}",
                self.keyframes,
                if self.keyframes == 1 {
                    "keyframe"
                } else {
                    "keyframes"
                }
            ),
        ];
        if issue_count > 0 {
            left.push("Editor slowed down".into());
        }
        left.extend(self.extension_items.iter().map(|i| i.text.clone()));
        StatusText {
            left,
            right: vec![
                if self.playing { "Playing" } else { "Paused" }.into(),
                format!("{}×{}", self.width, self.height),
                format!("{} fps", self.rate),
                self.workspace.clone(),
            ],
            indicator_label: (issue_count > 0).then(|| match issue_count {
                1 => "Editor slowed down, 1 issue".to_string(),
                n => format!("Editor slowed down, {n} issues"),
            }),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    EditorThread,
    Compiling,
    Gpu,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Layer(String),
    Extension(String),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub milliseconds: f64,
    pub stage: Stage,
    pub target: Target,
    pub turn_off_failed: bool,
}
impl Issue {
    pub fn fact(&self) -> Option<String> {
        if !self.milliseconds.is_finite() || self.milliseconds < 0. {
            return None;
        }
        let where_ = match self.stage {
            Stage::EditorThread => "on the editor thread",
            Stage::Compiling => "compiling",
            Stage::Gpu => "on the GPU",
        };
        Some(format!(
            "{} ms {where_}. A 60 fps frame has 16.7 ms available.",
            self.milliseconds
        ))
    }
    pub fn button(&self) -> &'static str {
        match self.target {
            Target::Layer(_) => "Select Layer",
            Target::Extension(_) => "Turn Off",
        }
    }
    pub fn error(&self) -> Option<String> {
        (self.turn_off_failed && matches!(self.target, Target::Extension(_))).then(|| {
            format!(
                "Couldn’t turn off {}. Try again in Settings ▸ Extensions.",
                self.name
            )
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlowdownAction {
    SelectLayer(String),
    TurnOff(String),
}
#[derive(Default)]
pub struct Slowdown {
    pub open: bool,
    pub issues: Vec<Issue>,
}
impl Slowdown {
    pub const TITLE: &'static str = "What’s Slowing the Editor Down";
    pub fn update(&mut self, issues: Vec<Issue>) {
        self.issues = issues;
        if self.issues.is_empty() {
            self.open = false;
        }
    }
    pub fn show(&mut self) {
        self.open = !self.issues.is_empty();
    }
    pub fn activate(&self, id: &str, revision: u64) -> Option<SlowdownAction> {
        let issue = self
            .issues
            .iter()
            .find(|i| i.id == id && i.revision == revision)?;
        if !self.open {
            return None;
        }
        // Selection deliberately leaves the NSPopover open and emits no toast.
        Some(match &issue.target {
            Target::Layer(id) => SlowdownAction::SelectLayer(id.clone()),
            Target::Extension(id) => SlowdownAction::TurnOff(id.clone()),
        })
    }
    pub fn turn_off_completed(&mut self, id: &str, revision: u64, success: bool) {
        if let Some(issue) = self.issues.iter_mut().find(|i| {
            i.id == id && i.revision == revision && matches!(i.target, Target::Extension(_))
        }) {
            issue.turn_off_failed = !success;
        }
    }
}
