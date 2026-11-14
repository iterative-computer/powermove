//! N004–N006: docs/native/design/surfaces/extensions.md. Bringing the original
//! app's extensions into a library that already exists, at startup or from
//! Settings › Storage. Headless: the host maps this to its Welcome frame.
use super::{Button, View, migration::StepRow};
use crate::extension_transfer::{Progress, Scan, ScanStatus, TransferOutcome, TransferResult};

/// Where the step was opened. Startup always ends in the library; Settings
/// ends where it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Startup,
    Settings,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// N004: what was found, and the choice.
    Offer,
    /// N005: copying and checking, one extension at a time.
    Running,
    /// N006: what came in, what was already here, what couldn't load.
    Done,
    /// N006: the transfer stopped before it finished; nothing else changed.
    Failed,
}
/// One extension's line in a list: its tone and its sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    /// Couldn't load: copied, but it won't run until it's fixed.
    Warning,
    /// Skipped or already here: nothing to do.
    Note,
    /// Brought in and loads.
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Copying,
    Checking,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub tone: Tone,
    /// `id: reason`, a whole sentence that wraps inside its card.
    pub text: String,
}
#[derive(Debug, Clone)]
pub struct Model {
    pub place: Place,
    pub gate: Gate,
    pub scan: Scan,
    pub progress: Option<Progress>,
    pub result: Option<TransferResult>,
    /// Why the transfer stopped, for Details.
    pub error: Option<String>,
}
fn extensions(n: usize) -> String {
    match n {
        1 => "1 extension".into(),
        n => format!("{n} extensions"),
    }
}
fn item(id: &str, tone: Tone, reason: &str) -> Item {
    Item {
        id: id.into(),
        tone,
        text: if reason.is_empty() {
            id.into()
        } else {
            format!("{id}: {reason}")
        },
    }
}
const UNTOUCHED: &str =
    "Only extensions are copied. Your projects, settings and sign-ins stay as they are.";
impl Model {
    pub fn offer(scan: Scan, place: Place) -> Self {
        Self {
            place,
            gate: Gate::Offer,
            scan,
            progress: None,
            result: None,
            error: None,
        }
    }
    /// New and updated: what Bring In Extensions copies.
    pub fn offered(&self) -> usize {
        self.scan.new + self.scan.changed
    }
    pub fn skipped(&self) -> usize {
        self.scan
            .entries
            .iter()
            .filter(|e| e.status == ScanStatus::Skipped && !e.id.starts_with('.'))
            .count()
    }
    pub fn start(&mut self) {
        self.gate = Gate::Running;
        self.progress = None;
        self.result = None;
        self.error = None;
    }
    pub fn progress(&mut self, progress: Progress) {
        if self.gate == Gate::Running {
            self.progress = Some(progress);
        }
    }
    pub fn finished(&mut self, result: Result<TransferResult, String>) {
        match result {
            Ok(result) => {
                self.gate = Gate::Done;
                self.result = Some(result);
            }
            Err(error) => {
                self.gate = Gate::Failed;
                self.error = Some(error);
            }
        }
    }
    /// The engine copies every extension, then checks each one: `done` runs
    /// over twice the entries. The phase, and how far into it (1-based).
    pub fn phase(&self) -> Option<(Phase, usize, usize)> {
        let p = self.progress.as_ref().filter(|p| p.total > 0)?;
        let half = p.total.div_ceil(2);
        Some(if p.done < half {
            (Phase::Copying, p.done + 1, half)
        } else {
            (
                Phase::Checking,
                (p.done - half + 1).min(p.total - half),
                p.total - half,
            )
        })
    }
    pub fn view(&self) -> View {
        let startup = self.place == Place::Startup;
        let (surface, title, line, status, buttons, default): (
            &'static str,
            String,
            String,
            Option<String>,
            Vec<Button>,
            Option<&'static str>,
        ) = match self.gate {
            Gate::Offer => (
                "N004",
                "Bring In Your Extensions".into(),
                format!(
                    "The original app has {} that {} in Powermove yet. {UNTOUCHED}",
                    extensions(self.offered()),
                    if self.offered() == 1 {
                        "isn’t"
                    } else {
                        "aren’t"
                    }
                ),
                None,
                vec![
                    Button::new(
                        if startup { "not_now" } else { "cancel" },
                        if startup { "Not Now" } else { "Cancel" },
                    ),
                    Button {
                        id: "bring_in",
                        label: "Bring In Extensions",
                        enabled: self.offered() > 0,
                    },
                ],
                Some("bring_in"),
            ),
            Gate::Running => (
                "N005",
                "Bringing In Your Extensions".into(),
                UNTOUCHED.into(),
                Some(match (&self.progress, self.phase()) {
                    (Some(p), Some((Phase::Copying, _, _))) if !p.current.is_empty() => {
                        format!("Copying {}…", p.current)
                    }
                    (Some(p), Some((Phase::Checking, _, _))) if !p.current.is_empty() => {
                        format!("Checking that {} loads…", p.current)
                    }
                    _ => "Getting ready…".into(),
                }),
                vec![],
                None,
            ),
            Gate::Done => {
                let r = self.result.clone().unwrap_or_default();
                let mut parts = vec![];
                if r.brought_in > 0 || r.already_present + r.could_not_load == 0 {
                    parts.push(format!("{} brought in", r.brought_in));
                }
                if r.already_present > 0 {
                    parts.push(format!("{} already in Powermove", r.already_present));
                }
                if r.could_not_load > 0 {
                    parts.push(format!("{} couldn’t load", r.could_not_load));
                }
                let mut counts = match parts.as_slice() {
                    [one] => one.clone(),
                    [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
                    [] => String::new(),
                };
                if let Some(first) = counts.get(..1) {
                    counts = first.to_uppercase() + &counts[1..];
                }
                (
                    "N006",
                    if r.brought_in == 0 && r.could_not_load > 0 {
                        "Your Extensions Couldn’t Load".into()
                    } else if r.brought_in == 0 {
                        "Nothing New to Bring In".into()
                    } else {
                        "Your Extensions Are In".into()
                    },
                    format!("{counts}. Your original app stays unchanged."),
                    None,
                    vec![if startup {
                        Button::new("open_native", "Open Powermove")
                    } else {
                        Button::new("done", "Done")
                    }],
                    Some(if startup { "open_native" } else { "done" }),
                )
            }
            Gate::Failed => (
                "N006",
                "Your Extensions Couldn’t Be Brought In".into(),
                if startup {
                    "Nothing in your library changed. You can open it now and try again later from Settings › Storage.".into()
                } else {
                    "Nothing in your library changed. Check that the original app’s folder is still there, then try again.".into()
                },
                None,
                if startup {
                    vec![
                        Button::new("try_again", "Try Again"),
                        Button::new("open_native", "Open Powermove"),
                    ]
                } else {
                    vec![
                        Button::new("done", "Close"),
                        Button::new("try_again", "Try Again"),
                    ]
                },
                Some(if startup { "open_native" } else { "try_again" }),
            ),
        };
        View {
            surface,
            title,
            line,
            status,
            buttons,
            default,
        }
    }
    /// The offer's counts card (a row per kind that has any), or the running bar.
    pub fn rows(&self) -> Vec<StepRow> {
        let row = |title: &str, n: usize| StepRow {
            title: title.into(),
            value: n.to_string(),
            active: None,
        };
        let mut rows = Vec::new();
        match self.gate {
            Gate::Offer => {
                rows.push(row("New", self.scan.new));
                rows.push(row("Updated in the original app", self.scan.changed));
                rows.push(row("Already in Powermove", self.scan.already_present));
                rows.push(row("Skipped", self.skipped()));
            }
            Gate::Running => {
                let Some(p) = self.progress.as_ref().filter(|p| p.total > 0) else {
                    return vec![StepRow {
                        title: "Extensions".into(),
                        value: String::new(),
                        active: Some(None),
                    }];
                };
                let (phase, at, of) = self.phase().unwrap_or((Phase::Copying, 0, 0));
                return vec![StepRow {
                    title: match phase {
                        Phase::Copying => "Copying",
                        Phase::Checking => "Checking",
                    }
                    .into(),
                    value: format!("{at} of {of}"),
                    active: Some(Some(p.done.min(p.total) as f64 / p.total as f64)),
                }];
            }
            // The line already gives the counts: the list gets the room.
            Gate::Done | Gate::Failed => {}
        }
        rows.retain(|r| r.value != "0");
        rows
    }
    /// Each extension with its reason. The offer lists only what it won't
    /// bring in; the summary lists every extension, problems first.
    pub fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = match self.gate {
            Gate::Offer => self
                .scan
                .entries
                .iter()
                .filter(|e| e.status == ScanStatus::Skipped)
                .map(|e| item(&e.id, Tone::Note, &e.reason))
                .collect(),
            Gate::Done => self
                .result
                .iter()
                .flat_map(|r| &r.entries)
                .map(|e| {
                    let tone = match e.outcome {
                        TransferOutcome::CouldNotLoad => Tone::Warning,
                        TransferOutcome::BroughtIn => Tone::Done,
                        TransferOutcome::AlreadyPresent | TransferOutcome::Skipped => Tone::Note,
                    };
                    item(&e.id, tone, &e.reason)
                })
                .collect(),
            Gate::Running | Gate::Failed => vec![],
        };
        items.retain(|i| !i.id.starts_with('.'));
        // Stable: within a tone, the engine's (alphabetical) order.
        items.sort_by_key(|i| i.tone);
        items
    }
    /// Startup only: where to find this again after Not Now.
    pub fn note(&self) -> Option<&'static str> {
        (self.place == Place::Startup && self.gate == Gate::Offer)
            .then_some("Choose Not Now to open your library as it is. You can bring them in later from Settings › Storage.")
    }
    pub fn details(&self) -> Option<&str> {
        self.error.as_deref().filter(|_| self.gate == Gate::Failed)
    }
}
