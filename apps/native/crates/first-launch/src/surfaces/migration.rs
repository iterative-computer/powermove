//! N001–N003: design/surfaces/migration.md. Gate projections carry no private
//! names, paths or secret counts. Report data is a separate, private capability.
use super::{Button, View};
use crate::{Event, Outcome, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    WaitingLocal,
    WaitingUnknown,
    Copying,
    Resuming,
    /// Asked to stop; the worker is still running to its next durable boundary.
    Pausing,
    Paused,
    LowDisk,
    Failed,
    ProfileRefused,
    Consent,
    InterruptedConsent,
    WaitingForMacos,
    Denied,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Permission,
    InvalidData,
    Unknown,
}
impl Failure {
    pub fn informative(self) -> &'static str {
        match self {
            Self::Permission => "Check its sharing and permissions in Finder.",
            Self::InvalidData => "The file may be damaged.",
            Self::Unknown => "Try again. If it keeps happening, the Details below can help.",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub step: Step,
    pub done: Option<usize>,
    pub total: Option<usize>,
}
impl Progress {
    pub fn status(&self) -> String {
        let counts = self
            .done
            .zip(self.total)
            .filter(|(done, total)| *total > 0 && done <= total);
        match (self.step, counts) {
            (Step::Library, Some((done, total))) => format!("Copying projects · {done} of {total}"),
            (Step::Media, Some((done, total))) => format!("Copying media · {done} of {total}"),
            (Step::Checkpoint, _) => "Taking a snapshot of your library…".into(),
            (Step::Bootstrap, _) => "Preparing your library…".into(),
            // The spec defines count-bearing copy for these steps. With no
            // total, keep a neutral indeterminate status instead of new copy.
            (Step::Library | Step::Media, _) => String::new(),
            (Step::LocalStorage, _) => "Copying settings…".into(),
            (Step::Extensions, _) => "Copying extensions…".into(),
            (Step::Secrets, _) => "Waiting for macOS…".into(),
        }
    }
    /// Raw counts only: the host must not invent a percent without a total.
    pub fn determinate(&self) -> Option<(usize, usize)> {
        self.done
            .zip(self.total)
            .filter(|(done, total)| *total > 0 && done <= total)
    }
}
#[derive(Debug)]
pub struct Model {
    pub gate: Gate,
    /// Set by trusted lock-owner evidence; a boolean Waiting event alone cannot
    /// prove the owner is on this Mac. Unknown remains the conservative default.
    pub electron_owner_local: bool,
    pub progress: Option<Progress>,
    pub summary: Option<Summary>,
    /// Includes sign-ins not copied and kept-aside/conflict rows, without
    /// exposing a secret count on the gate.
    pub report_needs_review: bool,
    pub failure: Failure,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            gate: Gate::Copying,
            electron_owner_local: false,
            progress: None,
            summary: None,
            report_needs_review: false,
            failure: Failure::Unknown,
        }
    }
}
impl Model {
    pub fn event(&mut self, event: &Event) {
        match event {
            Event::Started(step) if self.gate == Gate::Pausing => {
                self.progress = Some(Progress {
                    step: *step,
                    done: None,
                    total: None,
                });
            }
            Event::WaitingForElectron | Event::SecretPromptRequired
                if self.gate == Gate::Pausing => {}
            Event::WaitingForElectron => {
                self.gate = if self.electron_owner_local {
                    Gate::WaitingLocal
                } else {
                    Gate::WaitingUnknown
                }
            }
            Event::Started(step) => {
                self.gate = if *step == Step::Secrets {
                    Gate::WaitingForMacos
                } else {
                    Gate::Copying
                };
                self.progress = Some(Progress {
                    step: *step,
                    done: None,
                    total: None,
                });
            }
            Event::Progress { step, done, total } => {
                self.progress = Some(Progress {
                    step: *step,
                    done: Some(*done),
                    total: Some(*total),
                })
            }
            Event::SecretPromptRequired => self.gate = Gate::Consent,
            Event::Published(_) | Event::Complete => {}
        }
    }
    pub fn outcome(&mut self, outcome: &Outcome) {
        match outcome {
            Outcome::WaitingForElectron => {
                self.gate = if self.electron_owner_local {
                    Gate::WaitingLocal
                } else {
                    Gate::WaitingUnknown
                }
            }
            Outcome::NeedsSecretConsent { interrupted } => {
                self.gate = if *interrupted {
                    Gate::InterruptedConsent
                } else {
                    Gate::Consent
                }
            }
            Outcome::Complete {
                projects,
                history_refusals,
                missing_projects,
                blocked_owners,
                proxy_only,
                already_offline,
            } => {
                self.gate = Gate::Complete;
                self.summary = Some(Summary {
                    projects: *projects,
                    history_refusals: *history_refusals,
                    missing_projects: *missing_projects,
                    blocked_owners: *blocked_owners,
                    proxy_only: *proxy_only,
                    already_offline: *already_offline,
                });
            }
        }
    }
    /// Host pauses only at the next durable callback boundary, preserving the
    /// active checkpoint. This does not abort a publication half way through.
    pub fn pause_at_boundary(&mut self) {
        if matches!(self.gate, Gate::Copying | Gate::Resuming) {
            self.gate = Gate::Pausing;
        }
    }
    pub fn resume(&mut self) {
        if self.gate == Gate::Paused {
            self.gate = Gate::Resuming;
        }
    }
    pub fn view(&self) -> View {
        let (title,line,status,buttons,default) = match self.gate {
            Gate::WaitingLocal => ("Quit Powermove to Continue", "Powermove Native copies your library while the original app is closed. Save your work there and quit it.".into(),Some("Waiting for Powermove to quit…".into()),vec![],None),
            Gate::WaitingUnknown => ("Powermove Might Still Be Open","It looks like the original app is using this library, possibly on another Mac. Close it there, then check again.".into(),None,vec![Button::new("check_again","Check Again")],Some("check_again")),
            Gate::Copying|Gate::Resuming => ("Copying Your Library","Your original app stays unchanged, and this copy never replaces it.".into(),if self.gate==Gate::Resuming {Some("Picking up where it stopped…".into())} else {self.progress.as_ref().map(Progress::status)},vec![Button::new("pause","Pause")],None),
            Gate::Pausing => ("Copying Your Library","Your original app stays unchanged, and this copy never replaces it.".into(),Some("Pausing…".into()),vec![Button{id:"pause",label:"Pause",enabled:false}],None),
            Gate::Paused => ("Copying Is Paused","It picks up where it stopped. You can quit and come back.".into(),self.progress.as_ref().map(Progress::status),vec![Button::new("resume","Resume")],Some("resume")),
            Gate::LowDisk => ("Not Enough Space",String::new(),None,vec![Button::new("check_again","Check Again")],Some("check_again")),
            Gate::Failed => ("Copying Stopped",format!("Your original app and library weren’t changed. {}",self.failure.informative()),None,vec![Button::new("try_again","Try Again")],Some("try_again")),
            Gate::ProfileRefused => ("Powermove Native Can’t Start","Its library folder isn’t usable. Your original app wasn’t changed.".into(),None,vec![],Some("quit")),
            Gate::Consent|Gate::InterruptedConsent => (if self.gate==Gate::Consent {"Keep Your Sign-Ins"} else {"The Keychain Request Didn’t Finish"},if self.gate==Gate::Consent {crate::secrets::PROMPT.into()} else {"Powermove Native stopped before macOS answered. Continuing asks macOS one more time. Your original app stays unchanged.".into()},None,vec![Button::new("skip_secret_consent","Not Now"),Button::new("resume_with_secret_consent","Continue")],Some("resume_with_secret_consent")),
            Gate::WaitingForMacos => ("Keep Your Sign-Ins",crate::secrets::PROMPT.into(),Some("Waiting for macOS…".into()),vec![Button{id:"skip_secret_consent",label:"Not Now",enabled:false},Button{id:"resume_with_secret_consent",label:"Continue",enabled:false}],None),
            Gate::Denied => ("Keychain Access Wasn’t Allowed","Your sign-ins and extension values stay in the original app. You can sign in again here, or try again.".into(),None,vec![Button::new("skip_secret_consent","Continue Without Them"),Button::new("resume_with_secret_consent","Try Again")],Some("skip_secret_consent")),
            Gate::Complete => ("Your Library Is Ready",format!("{} Your original app stays unchanged.",copied(self.summary.as_ref().map_or(0,|s|s.projects))),None,if self.report_needs_review || self.summary.as_ref().is_some_and(|s|!s.rows().is_empty()) {vec![Button::new("show_report","Show Report…"),Button::new("open_native","Open Powermove Native")]} else {vec![Button::new("open_native","Open Powermove Native")]},Some("open_native")),
        };
        let mut buttons = buttons;
        if !matches!(self.gate, Gate::Complete | Gate::WaitingForMacos) {
            buttons.insert(0, Button::new("quit", "Quit Powermove Native"));
        }
        View {
            surface: if matches!(
                self.gate,
                Gate::Consent | Gate::InterruptedConsent | Gate::WaitingForMacos | Gate::Denied
            ) {
                "N002"
            } else if self.gate == Gate::Complete {
                "N003"
            } else {
                "N001"
            },
            title: title.into(),
            line,
            status,
            buttons,
            default,
        }
    }
    pub fn report(&mut self, report: &Report) {
        self.report_needs_review = !report.rows.is_empty();
    }
    pub fn details_available(&self) -> bool {
        matches!(self.gate, Gate::Failed | Gate::ProfileRefused)
    }
    pub fn low_disk_view(&self, needed: &str, available: &str) -> View {
        let mut view = self.view();
        if self.gate == Gate::LowDisk {
            view.line = format!(
                "Powermove Native needs {needed} free to copy your library, and this Mac has {available}. Nothing was deleted."
            );
        }
        view
    }
}
fn copied(n: usize) -> String {
    match n {
        1 => "1 project was copied.".into(),
        n => format!("{n} projects were copied."),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub projects: usize,
    pub history_refusals: usize,
    pub missing_projects: usize,
    pub blocked_owners: usize,
    pub proxy_only: usize,
    pub already_offline: usize,
}
impl Summary {
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let projects = |n: usize, one: &str, many: &str| match n {
            1 => format!("1 project {one}"),
            n => format!("{n} projects {many}"),
        };
        let media = |n: usize, one: &str, many: &str| match n {
            1 => format!("1 media file {one}"),
            n => format!("{n} media files {many}"),
        };
        [
            (
                "history_refusals",
                self.history_refusals,
                projects(
                    self.history_refusals,
                    "came across without its Undo history. Its current state is complete.",
                    "came across without their Undo history. Their current state is complete.",
                ),
            ),
            (
                "missing_projects",
                self.missing_projects,
                projects(
                    self.missing_projects,
                    "couldn’t be read. It’s kept safely and wasn’t changed.",
                    "couldn’t be read. They’re kept safely and weren’t changed.",
                ),
            ),
            (
                "blocked_owners",
                self.blocked_owners,
                projects(
                    self.blocked_owners,
                    "needs media that went missing. It’ll ask you to find it.",
                    "need media that went missing. They’ll ask you to find it.",
                ),
            ),
            (
                "proxy_only",
                self.proxy_only,
                media(
                    self.proxy_only,
                    "came across as a proxy only. Exports using it show a warning until you relink the original.",
                    "came across as proxies only. Exports using them show a warning until you relink the originals.",
                ),
            ),
            (
                "already_offline",
                self.already_offline,
                media(
                    self.already_offline,
                    "was already offline.",
                    "were already offline.",
                ),
            ),
        ]
        .into_iter()
        .filter(|(_, n, _)| *n > 0)
        .map(|(id, _, text)| (id, text))
        .collect()
    }
}
pub const PROXY_EXPORT_WARNING: &str =
    "This uses proxy-only media, so quality may be lower. Relink the originals for full quality.";
pub const REPORT_SECTIONS: [(&str, &str); 7] = [
    ("Projects Without Undo History", "Open"),
    ("Projects That Couldn’t Be Read", "Show in Finder"),
    ("Waiting for Media", "Relink…"),
    ("Proxy-Only Media", "Relink…"),
    ("Kept Aside", "Show in Finder"),
    ("Conflicts", "Show in Finder"),
    ("Sign-Ins Not Copied", "Set Up…"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
pub enum ReportKind {
    History,
    MissingProjects,
    Media,
    Proxies,
    Quarantined,
    Conflicts,
    SignIns,
}
impl ReportKind {
    pub fn section(self) -> (&'static str, &'static str) {
        REPORT_SECTIONS[match self {
            Self::History => 0,
            Self::MissingProjects => 1,
            Self::Media => 2,
            Self::Proxies => 3,
            Self::Quarantined => 4,
            Self::Conflicts => 5,
            Self::SignIns => 6,
        }]
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    pub id: String,
    pub display_name: String,
    pub kind: ReportKind,
}
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub rows: Vec<ReportRow>,
}
impl Report {
    pub fn sections(&self) -> std::collections::BTreeMap<ReportKind, Vec<&ReportRow>> {
        let mut sections = std::collections::BTreeMap::new();
        for row in &self.rows {
            sections.entry(row.kind).or_insert_with(Vec::new).push(row);
        }
        sections
    }
    pub fn empty_copy(&self) -> Option<(&'static str, &'static str)> {
        self.rows
            .is_empty()
            .then_some(("Nothing to Review", "Everything came across."))
    }
    /// Current Opus spec: informational explanation only. No reverse-migration
    /// command, return path, or guaranteed native-file compatibility.
    pub fn footer_actions(&self) -> Vec<Button> {
        vec![
            Button::new("copy_report", "Copy Report"),
            Button::new("show_library", "Show Library in Finder"),
            Button::new("about_native_files", "About Going Back…"),
        ]
    }
}
pub fn about_native_files() -> View {
    View{surface:"N003",title:"Your original app still has everything from before.".into(),line:"Work you do in Powermove Native doesn’t move back to it. It can open a .pmv saved here only if it can read everything in the file. If it can’t, it refuses the file, and the file isn’t changed.".into(),status:None,buttons:vec![Button::new("ok","OK")],default:Some("ok")}
}
pub fn blocked_project_view(project: &str) -> View {
    View{surface:"N003",title:format!("Media for “{project}” is missing."),line:"Powermove Native can’t open this copy until the media is found. The original project wasn’t changed.".into(),status:None,buttons:vec![Button::new("relink","Relink…"),Button::new("cancel","Cancel")],default:Some("relink")}
}
/// Offline and proxy-only inputs remain openable; only a blocked owner is gated.
pub fn project_open_allowed(blocked_owner: bool) -> bool {
    !blocked_owner
}
