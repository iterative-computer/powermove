//! N004–N006: docs/native/design/surfaces/extensions.md. Bringing the original
//! app's extensions into a library that already exists, at startup or from
//! Settings › Storage. Headless: the host maps this to its Welcome frame.
//!
//! Each screen leads with the outcome (a sentence and its counts), shows the
//! exceptions in full with their reasons, and lists the rest by name only.
use super::{Button, View, migration::StepRow};
use crate::extension_transfer::{Progress, Scan, ScanStatus, TransferOutcome, TransferResult};
use std::collections::BTreeMap;

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
/// An exception's tone, most serious first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    /// Couldn't load: copied, but it won't run until it's fixed.
    Warning,
    /// Skipped or already here: nothing to do.
    Note,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Copying,
    Checking,
}
/// An extension that didn't simply come in: its name, and why, in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub tone: Tone,
    pub reason: String,
}
#[derive(Debug, Clone)]
pub struct Model {
    pub place: Place,
    pub gate: Gate,
    pub scan: Scan,
    /// Display names from the extensions' manifests, by folder id.
    pub names: BTreeMap<String, String>,
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
const UNTOUCHED: &str =
    "Only extensions are copied. Your projects, settings and sign-ins stay as they are.";
/// Names shown before Show All: three rows of two.
pub const PREVIEW: usize = 6;
impl Model {
    pub fn offer(scan: Scan, place: Place) -> Self {
        Self {
            place,
            gate: Gate::Offer,
            scan,
            names: BTreeMap::new(),
            progress: None,
            result: None,
            error: None,
        }
    }
    pub fn named(mut self, names: BTreeMap<String, String>) -> Self {
        self.names = names;
        self
    }
    /// The manifest's name, else the folder's.
    pub fn name(&self, id: &str) -> String {
        self.names
            .get(id)
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .map_or_else(|| id.to_owned(), str::to_owned)
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
    fn skipped_in_result(&self) -> usize {
        self.result.as_ref().map_or(0, |r| {
            r.entries
                .iter()
                .filter(|e| e.outcome == TransferOutcome::Skipped && !e.id.starts_with('.'))
                .count()
        })
    }
    pub fn view(&self) -> View {
        let startup = self.place == Place::Startup;
        let (surface, title, line, buttons, default): (
            &'static str,
            String,
            String,
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
                vec![],
                None,
            ),
            Gate::Done => {
                let r = self.result.clone().unwrap_or_default();
                let title = if r.brought_in == 0 && r.could_not_load > 0 {
                    "Your Extensions Couldn’t Load"
                } else if r.brought_in == 0 {
                    "Nothing New to Bring In"
                } else {
                    "Your Extensions Are In"
                };
                let lead = match (r.brought_in, r.could_not_load) {
                    (0, 0) => "They were already in Powermove.".to_owned(),
                    (_, 0) => "Each one was checked and loads.".to_owned(),
                    (_, 1) => "One couldn’t load. The reason is below.".to_owned(),
                    (_, n) => format!("{n} couldn’t load. The reasons are below."),
                };
                (
                    "N006",
                    title.into(),
                    format!("{lead} Your original app stays unchanged."),
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
            status: None,
            buttons,
            default,
        }
    }
    /// The counts that lead the offer and the summary: each kind that has any.
    pub fn stats(&self) -> Vec<(usize, &'static str)> {
        let mut stats = match self.gate {
            Gate::Offer => vec![
                (self.scan.new, "New"),
                (self.scan.changed, "Updated"),
                (self.scan.already_present, "Already in Powermove"),
                (self.skipped(), "Skipped"),
            ],
            Gate::Done => {
                let r = self.result.clone().unwrap_or_default();
                vec![
                    (r.brought_in, "Brought in"),
                    (r.could_not_load, "Couldn’t load"),
                    (r.already_present, "Already in Powermove"),
                    (self.skipped_in_result(), "Skipped"),
                ]
            }
            Gate::Running | Gate::Failed => vec![],
        };
        stats.retain(|(n, _)| *n > 0);
        stats
    }
    /// The running row: what's happening, to which extension, how far.
    pub fn rows(&self) -> Vec<StepRow> {
        if self.gate != Gate::Running {
            return vec![];
        }
        let Some(p) = self.progress.as_ref().filter(|p| p.total > 0) else {
            return vec![StepRow {
                title: "Getting ready".into(),
                value: String::new(),
                active: Some(None),
            }];
        };
        let (phase, _, _) = self.phase().unwrap_or((Phase::Copying, 0, 0));
        vec![StepRow {
            title: match phase {
                Phase::Copying => "Copying",
                Phase::Checking => "Checking that it loads",
            }
            .into(),
            value: self.name(&p.current),
            active: Some(Some(p.done.min(p.total) as f64 / p.total as f64)),
        }]
    }
    /// What didn't simply come in, in full: couldn't load first, then
    /// skipped, then already here.
    pub fn items(&self) -> Vec<Item> {
        let item = |id: &str, tone, reason: &str| Item {
            id: id.into(),
            name: self.name(id),
            tone,
            reason: reason.into(),
        };
        let mut items: Vec<(Item, bool)> = match self.gate {
            Gate::Offer => self
                .scan
                .entries
                .iter()
                .filter(|e| e.status == ScanStatus::Skipped)
                .map(|e| (item(&e.id, Tone::Note, &e.reason), false))
                .collect(),
            Gate::Done => self
                .result
                .iter()
                .flat_map(|r| &r.entries)
                .filter_map(|e| match e.outcome {
                    TransferOutcome::BroughtIn => None,
                    TransferOutcome::CouldNotLoad => {
                        Some((item(&e.id, Tone::Warning, &e.reason), false))
                    }
                    TransferOutcome::Skipped => Some((item(&e.id, Tone::Note, &e.reason), false)),
                    TransferOutcome::AlreadyPresent => {
                        Some((item(&e.id, Tone::Note, &e.reason), true))
                    }
                })
                .collect(),
            Gate::Running | Gate::Failed => vec![],
        };
        items.retain(|(i, _)| !i.id.starts_with('.'));
        // Stable: within a tone, already-here after skipped, then the
        // engine's (alphabetical) order.
        items.sort_by_key(|(i, already)| (i.tone, *already));
        items.into_iter().map(|(i, _)| i).collect()
    }
    pub fn items_title(&self) -> Option<&'static str> {
        let items = self.items();
        if items.is_empty() {
            None
        } else if items.iter().any(|i| i.tone == Tone::Warning) {
            Some("Needs a Look")
        } else {
            Some("Not Brought In")
        }
    }
    /// Everything that comes in (the offer) or came in (the summary), by
    /// name only, sorted as a person reads them.
    pub fn listed(&self) -> Option<(&'static str, Vec<String>)> {
        let mut names: Vec<String> = match self.gate {
            Gate::Offer => self
                .scan
                .entries
                .iter()
                .filter(|e| matches!(e.status, ScanStatus::New | ScanStatus::Changed))
                .map(|e| self.name(&e.id))
                .collect(),
            Gate::Done => self
                .result
                .iter()
                .flat_map(|r| &r.entries)
                .filter(|e| e.outcome == TransferOutcome::BroughtIn)
                .map(|e| self.name(&e.id))
                .collect(),
            Gate::Running | Gate::Failed => vec![],
        };
        if names.is_empty() {
            return None;
        }
        names.sort_by_key(|n| n.to_lowercase());
        Some((
            if self.gate == Gate::Offer {
                "To Bring In"
            } else {
                "Brought In"
            },
            names,
        ))
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
