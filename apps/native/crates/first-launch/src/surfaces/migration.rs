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
    /// Originals are in folders macOS protects: explain its prompt first.
    FolderAccess,
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
        // Counts and bytes are in the step rows; the status says what's happening.
        match self.step {
            Step::Checkpoint => "Taking a snapshot of your library…",
            Step::Inventory => "Checking your media…",
            Step::Bootstrap => "Preparing your library…",
            Step::Library => "Copying projects…",
            Step::Media => "Copying media…",
            Step::LocalStorage => "Copying settings…",
            Step::Extensions => "Copying extensions…",
            Step::Secrets => "Waiting for macOS…",
        }
        .into()
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
    /// Steps already published, for the step rows.
    pub finished: std::collections::BTreeSet<Step>,
    /// Live bytes of the running step (done, total; total 0 = unknown), polled.
    pub bytes: Option<(u64, u64)>,
    /// A macOS folder prompt is unanswered: the folder's Finder name, polled.
    pub waiting_folder: Option<String>,
    /// Folders macOS protects that hold originals, with file counts.
    pub folders: Vec<(String, usize)>,
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
            finished: Default::default(),
            bytes: None,
            waiting_folder: None,
            folders: Vec::new(),
        }
    }
}
impl Model {
    pub fn event(&mut self, event: &Event) {
        match event {
            Event::Published(step) => {
                self.finished.insert(*step);
            }
            Event::Started(step) if self.gate == Gate::Pausing => {
                self.progress = Some(Progress {
                    step: *step,
                    done: None,
                    total: None,
                });
            }
            Event::WaitingForElectron
            | Event::SecretPromptRequired
            | Event::FolderAccessRequired
                if self.gate == Gate::Pausing => {}
            Event::WaitingForElectron => {
                self.gate = if self.electron_owner_local {
                    Gate::WaitingLocal
                } else {
                    Gate::WaitingUnknown
                }
            }
            Event::Started(step) => {
                self.bytes = None;
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
            Event::FolderAccessRequired => self.gate = Gate::FolderAccess,
            Event::Complete => {}
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
            Outcome::NeedsFolderAccess { folders } => {
                self.gate = Gate::FolderAccess;
                self.folders = folders.clone();
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
                not_reached,
            } => {
                self.gate = Gate::Complete;
                self.summary = Some(Summary {
                    projects: *projects,
                    history_refusals: *history_refusals,
                    missing_projects: *missing_projects,
                    blocked_owners: *blocked_owners,
                    proxy_only: *proxy_only,
                    already_offline: *already_offline,
                    not_reached: *not_reached,
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
        const UNCHANGED: &str =
            "Your original app stays unchanged, and this copy never replaces it.";
        let waiting_folder = self
            .waiting_folder
            .as_ref()
            .filter(|_| matches!(self.gate, Gate::Copying | Gate::Resuming | Gate::Pausing));
        let (title, line, status, buttons, default): (String, String, Option<String>, Vec<Button>, Option<&'static str>) = match self.gate {
            _ if let Some(folder) = waiting_folder => ("Waiting for macOS".into(), format!("macOS is asking whether Powermove can open your files in {folder}. Choose Allow in its dialog. If you don’t see it, it may be behind this window."), Some("Waiting for your answer…".into()), vec![Button::new("skip_folder", "Skip These Files")], None),
            Gate::WaitingLocal => ("Quit the Original App to Continue".into(), "Powermove copies your library while the original app is closed. Save your work there and quit it.".into(), Some("Waiting for the original app to quit…".into()), vec![], None),
            Gate::WaitingUnknown => ("The Original App Might Still Be Open".into(), "It looks like the original app is using this library, possibly on another Mac. Close it there, then check again.".into(), None, vec![Button::new("check_again", "Check Again")], Some("check_again")),
            Gate::Copying | Gate::Resuming => ("Copying Your Library".into(), UNCHANGED.into(), if self.gate == Gate::Resuming { Some("Picking up where it stopped…".into()) } else { self.progress.as_ref().map(Progress::status) }, vec![Button::new("pause", "Pause")], None),
            Gate::Pausing => ("Copying Your Library".into(), UNCHANGED.into(), Some("Pausing…".into()), vec![Button { id: "pause", label: "Pause", enabled: false }], None),
            Gate::Paused => ("Copying Is Paused".into(), "It picks up where it stopped. You can quit and come back.".into(), self.progress.as_ref().map(Progress::status), vec![Button::new("resume", "Resume")], Some("resume")),
            Gate::FolderAccess => ("Allow Access to Your Media".into(), format!("Some of your original media is in {}, which macOS protects. When you continue, macOS asks whether Powermove can open {}. Choose Allow.", folder_names(&self.folders), if self.folders.len() == 1 { "it" } else { "them" }), None, vec![Button::new("skip_folders", "Skip These Files"), Button::new("allow_folders", "Continue")], Some("allow_folders")),
            Gate::LowDisk => ("Not Enough Space".into(), String::new(), None, vec![Button::new("check_again", "Check Again")], Some("check_again")),
            Gate::Failed => ("Copying Stopped".into(), format!("Your original app and library weren’t changed. {}", self.failure.informative()), None, vec![Button::new("try_again", "Try Again")], Some("try_again")),
            Gate::ProfileRefused => ("Powermove Can’t Start".into(), "Its library folder isn’t usable. Your original app wasn’t changed.".into(), None, vec![], Some("quit")),
            Gate::Consent | Gate::InterruptedConsent => (if self.gate == Gate::Consent { "Keep Your Sign-Ins".into() } else { "The Keychain Request Didn’t Finish".into() }, if self.gate == Gate::Consent { crate::secrets::PROMPT.into() } else { "Powermove stopped before macOS answered. Continuing asks macOS one more time. Your original app stays unchanged.".into() }, None, vec![Button::new("skip_secret_consent", "Not Now"), Button::new("resume_with_secret_consent", "Continue")], Some("resume_with_secret_consent")),
            Gate::WaitingForMacos => ("Keep Your Sign-Ins".into(), crate::secrets::PROMPT.into(), Some("Waiting for macOS…".into()), vec![Button { id: "skip_secret_consent", label: "Not Now", enabled: false }, Button { id: "resume_with_secret_consent", label: "Continue", enabled: false }], None),
            Gate::Denied => ("Keychain Access Wasn’t Allowed".into(), "Your sign-ins and extension values stay in the original app. You can sign in again here, or try again.".into(), None, vec![Button::new("skip_secret_consent", "Continue Without Them"), Button::new("resume_with_secret_consent", "Try Again")], Some("skip_secret_consent")),
            Gate::Complete => ("Your Library Is Ready".into(), format!("{} Your original app stays unchanged.", copied(self.summary.as_ref().map_or(0, |s| s.projects))), None, if self.report_needs_review || self.summary.as_ref().is_some_and(|s| !s.rows().is_empty()) { vec![Button::new("show_report", "Show Report…"), Button::new("open_native", "Open Powermove")] } else { vec![Button::new("open_native", "Open Powermove")] }, Some("open_native")),
        };
        let mut buttons = buttons;
        if !matches!(self.gate, Gate::Complete | Gate::WaitingForMacos) {
            buttons.insert(0, Button::new("quit", "Quit Powermove"));
        }
        View {
            surface: if matches!(
                self.gate,
                Gate::Consent
                    | Gate::InterruptedConsent
                    | Gate::WaitingForMacos
                    | Gate::Denied
                    | Gate::FolderAccess
            ) || waiting_folder.is_some()
            {
                "N002"
            } else if self.gate == Gate::Complete {
                "N003"
            } else {
                "N001"
            },
            title,
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
                "Powermove needs {needed} free to copy your library, and this Mac has {available}. Nothing was deleted."
            );
        }
        view
    }
}
/// "Downloads", "Downloads and Desktop", "Downloads, Desktop and Documents".
fn folder_names(folders: &[(String, usize)]) -> String {
    let names: Vec<&str> = folders.iter().map(|(name, _)| name.as_str()).collect();
    match names.as_slice() {
        [] => "a folder".into(),
        [one] => (*one).into(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
fn files(n: usize) -> String {
    match n {
        1 => "1 file".into(),
        n => format!("{n} files"),
    }
}
/// Decimal units, as Finder shows sizes.
pub fn bytes(value: u64) -> String {
    let mut amount = value as f64;
    let mut unit = "bytes";
    for next in ["KB", "MB", "GB", "TB"] {
        if amount < 1000. {
            break;
        }
        amount /= 1000.;
        unit = next;
    }
    if unit == "bytes" {
        format!("{value} bytes")
    } else {
        format!("{amount:.1} {unit}")
    }
}
/// One row of the gate's card: what a step is, and how far it has got.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRow {
    pub title: String,
    /// Counts, bytes, `Done`, or empty for a step still to come.
    pub value: String,
    /// The running step: its fraction when known, else indeterminate.
    pub active: Option<Option<f64>>,
}
impl Model {
    /// The card under the explanation: each step and its progress, in order.
    /// Folder access lists each protected folder with its file count.
    pub fn rows(&self) -> Vec<StepRow> {
        if self.gate == Gate::FolderAccess {
            return self
                .folders
                .iter()
                .map(|(name, n)| StepRow {
                    title: name.clone(),
                    value: files(*n),
                    active: None,
                })
                .collect();
        }
        if !matches!(
            self.gate,
            Gate::Copying | Gate::Resuming | Gate::Pausing | Gate::Paused
        ) || self.waiting_folder.is_some()
        {
            return Vec::new();
        }
        let current = self.progress.as_ref().map(|p| p.step);
        let groups: [(&'static str, &[Step]); 5] = [
            ("Library snapshot", &[Step::Checkpoint]),
            ("Media check", &[Step::Inventory]),
            ("Projects", &[Step::Bootstrap, Step::Library]),
            ("Media", &[Step::Media]),
            (
                "Settings and extensions",
                &[Step::LocalStorage, Step::Extensions],
            ),
        ];
        let mut reached = current.is_none();
        groups
            .iter()
            .map(|(title, steps)| {
                let running = current.is_some_and(|c| steps.contains(&c));
                // Steps before the running one are done, even when this launch
                // resumed past them and never saw them publish.
                let done =
                    !running && (steps.iter().all(|s| self.finished.contains(s)) || !reached);
                if running {
                    reached = true;
                }
                let (value, active) = if running {
                    self.running_value()
                } else if done && current.is_some() {
                    ("Done".into(), None)
                } else {
                    (String::new(), None)
                };
                StepRow {
                    title: (*title).into(),
                    value,
                    active,
                }
            })
            .collect()
    }
    fn running_value(&self) -> (String, Option<Option<f64>>) {
        let counts = self.progress.as_ref().and_then(Progress::determinate);
        let bytes = self
            .bytes
            .filter(|(done, total)| *total > 0 && done <= total);
        let unit = match self.progress.as_ref().map(|p| p.step) {
            Some(Step::Inventory) => " projects",
            _ => "",
        };
        let count = counts.map(|(done, total)| format!("{done} of {total}{unit}"));
        let size =
            bytes.map(|(done, total)| format!("{} of {}", self::bytes(done), self::bytes(total)));
        let value = match (count, size) {
            (Some(count), Some(size)) => format!("{count} · {size}"),
            (Some(one), None) | (None, Some(one)) => one,
            (None, None) => String::new(),
        };
        // Bytes say more about time left than record counts do.
        let fraction = bytes
            .map(|(done, total)| done as f64 / total as f64)
            .or_else(|| counts.map(|(done, total)| done as f64 / total as f64));
        (value, Some(fraction))
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
    pub not_reached: usize,
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
                    "in the original app’s list had nothing saved, so there was nothing to bring over.",
                    "in the original app’s list had nothing saved, so there was nothing to bring over.",
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
                "not_reached",
                self.not_reached,
                media(
                    self.not_reached,
                    "wasn’t reached, because macOS didn’t allow access to its folder. It stays offline until you relink it.",
                    "weren’t reached, because macOS didn’t allow access to their folders. They stay offline until you relink them.",
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
pub const REPORT_SECTIONS: [(&str, &str); 8] = [
    ("Projects Without Undo History", "Open"),
    ("Projects With Nothing Saved", "Show in Finder"),
    ("Waiting for Media", "Relink…"),
    ("Proxy-Only Media", "Relink…"),
    ("Media Not Reached", "Relink…"),
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
    NotReached,
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
            Self::NotReached => 4,
            Self::Quarantined => 5,
            Self::Conflicts => 6,
            Self::SignIns => 7,
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
    View{surface:"N003",title:"Your original app still has everything from before.".into(),line:"Work you do in Powermove doesn’t move back to it. It can open a .pmv saved here only if it can read everything in the file. If it can’t, it refuses the file, and the file isn’t changed.".into(),status:None,buttons:vec![Button::new("ok","OK")],default:Some("ok")}
}
pub fn blocked_project_view(project: &str) -> View {
    View{surface:"N003",title:format!("Media for “{project}” is missing."),line:"Powermove can’t open this copy until the media is found. The original project wasn’t changed.".into(),status:None,buttons:vec![Button::new("relink","Relink…"),Button::new("cancel","Cancel")],default:Some("relink")}
}
/// Offline and proxy-only inputs remain openable; only a blocked owner is gated.
pub fn project_open_allowed(blocked_owner: bool) -> bool {
    !blocked_owner
}
