//! First launch: which path this launch takes, the migration worker, consent,
//! outcomes and onboarding. `Flow` is the whole state machine and owns no thread
//! or window, so every path is tested on synthetic profiles. `drive` runs it.
//!
//! Decisions: the app shares `pm_first_launch::default_paths` with the coordinator;
//! a clean outcome goes straight to the editor, because the normal path takes no
//! click; a later launch shows no gate unless merging runs past a second.
use anyhow::Result;
use pm_first_launch::{
    Config, DeltaPrepared, Event, KeychainDenied, Launch, LowDisk, Outcome, ProfileRefused,
    onboarding_completed,
    secrets::MacKeychain,
    source_access::{self, Activity, Choice as FolderChoice},
    surfaces::migration::{Failure, Gate, Model, Report, bytes},
};
use pm_kernel::locks::{ElectronOwner, electron_owner};
use pm_services::surface_backing::onboarding::{Choice as Chosen, Onboarding, Step as Onboard};
use pm_shell::welcome::{Button, Choice, Frame, Progress, Row, Severity, StepRow};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const APPLICATION_SUPPORT: &str = "Library/Application Support";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Absent on an override with no source: isolated dev and test profiles.
    pub config: Option<Config>,
    pub profile: PathBuf,
    pub launch: Launch,
    pub onboard: bool,
}
impl Plan {
    /// No override reads the shipped locations. An override never reaches them:
    /// a synthetic run names its source with `PM_NATIVE_ELECTRON_PROFILE`.
    pub fn resolve(
        profile: Option<PathBuf>,
        electron: Option<PathBuf>,
        home: Option<std::ffi::OsString>,
    ) -> Option<Self> {
        let support = home
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join(APPLICATION_SUPPORT));
        let isolated = profile.is_some();
        let defaults = support.as_deref().map(pm_first_launch::default_paths);
        let profile = profile.or_else(|| defaults.as_ref().map(|c| c.native_profile.clone()))?;
        let config = match (electron, isolated) {
            (Some(electron_profile), _) => Some(Config {
                electron_profile,
                native_profile: profile.clone(),
            }),
            (None, false) => defaults,
            (None, true) => None,
        };
        let launch = config
            .as_ref()
            .map_or(Launch::Fresh, pm_first_launch::detect);
        // A development review may walk onboarding on an isolated profile.
        let review =
            cfg!(debug_assertions) && std::env::var_os("PM_NATIVE_REVIEW_ONBOARDING").is_some();
        Some(Self {
            onboard: (!isolated || review) && !onboarding_completed(&profile),
            config,
            profile,
            launch,
        })
    }
    /// The source this launch must respect, for opens and locks.
    pub fn electron(&self) -> Option<PathBuf> {
        self.config
            .as_ref()
            .map(|c| c.electron_profile.clone())
            .filter(|p| p.is_dir())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Ordinary,
    WithConsent,
    SkipConsent,
    /// After the folder explanation: Continue (macOS then asks) or Skip These Files.
    Folders(FolderChoice),
}
#[derive(Clone, Debug)]
pub enum Finished {
    Outcome(Outcome),
    /// A finished copy and its report, read on the worker.
    Ready(Outcome, Result<Report, String>),
    Paused,
    LowDisk(LowDisk),
    ProfileRefused(String),
    Denied,
    Failed(Failure, String),
}
#[derive(Debug)]
pub enum Message {
    Event(Event),
    Finished(Finished),
    /// The waiting original app has gone; the copy can start.
    ElectronClosed,
}
/// Leaving the gate for the editor. `Enter` happens at most once per attempt.
#[derive(Debug)]
enum Entry {
    Waiting,
    Entering { updating: bool },
    Failed { details: String },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Run(Run),
    Pause,
    /// Watch the local original app until it quits.
    Watch,
    Quit,
    ShowReport,
    /// Stop waiting on the unanswered macOS folder prompt; those files stay offline.
    SkipFolder,
    /// Open the library and the editor, applying the choices if there are any.
    Enter(Option<serde_json::Value>),
}
#[derive(Debug)]
struct Choices {
    timeline: String,
    agent: String,
}
pub struct Flow {
    pub model: Model,
    plan: Plan,
    onboarding: Option<Onboarding>,
    choices: Choices,
    details: Option<String>,
    low_disk: Option<(String, String)>,
    report_unread: bool,
    error: Option<String>,
    consent_run: bool,
    entry: Entry,
    /// Onboarding's choices, kept so a failed attach can be tried again.
    chosen: Option<serde_json::Value>,
}
impl Flow {
    pub fn new(plan: Plan) -> Self {
        let mut model = Model::default();
        if let Some(config) = &plan.config {
            model.electron_owner_local =
                electron_owner(&config.electron_profile) == ElectronOwner::Local;
        }
        Self {
            model,
            onboarding: None,
            choices: Choices {
                timeline: "layers".into(),
                agent: "chatgpt".into(),
            },
            details: None,
            low_disk: None,
            report_unread: false,
            error: None,
            consent_run: false,
            entry: Entry::Waiting,
            chosen: None,
            plan,
        }
    }
    /// Where the launch starts.
    pub fn start(&mut self) -> Vec<Effect> {
        if self.plan.launch == Launch::Migrate {
            return vec![Effect::Run(Run::Ordinary)];
        }
        if self.plan.onboard {
            self.onboarding = Some(Onboarding::new(true));
            return vec![];
        }
        self.enter()
    }
    /// Whether this launch has a Welcome window to show.
    pub fn visible(&self) -> bool {
        self.plan.launch == Launch::Migrate
            || self.onboarding.is_some()
            || matches!(
                self.entry,
                Entry::Failed { .. } | Entry::Entering { updating: true }
            )
    }
    fn after_migration(&mut self) -> Vec<Effect> {
        if self.plan.onboard && !onboarding_completed(&self.plan.profile) {
            self.onboarding = Some(Onboarding::new(true));
            return vec![];
        }
        self.enter()
    }
    fn enter(&mut self) -> Vec<Effect> {
        self.entry = Entry::Entering { updating: false };
        vec![Effect::Enter(self.chosen.clone())]
    }
    /// The merge before the library attaches passed a second.
    pub fn updating(&mut self) {
        if let Entry::Entering { updating } = &mut self.entry {
            *updating = true;
        }
    }
    /// The library didn't open. The editor stays closed and the person can retry.
    pub fn enter_failed(&mut self, details: String) -> Vec<Effect> {
        self.entry = Entry::Failed { details };
        vec![]
    }
    /// The worker's live bytes and any unanswered folder prompt, polled. True if
    /// the frame changed.
    pub fn activity(&mut self, bytes: (u64, u64), waiting: Option<String>) -> bool {
        let bytes = Some(bytes).filter(|(_, total)| *total > 0);
        let changed = self.model.bytes != bytes || self.model.waiting_folder != waiting;
        self.model.bytes = bytes;
        self.model.waiting_folder = waiting;
        changed
    }
    pub fn message(&mut self, message: Message) -> Vec<Effect> {
        match message {
            Message::Event(event) => {
                self.model.event(&event);
                vec![]
            }
            Message::ElectronClosed => {
                self.model.electron_owner_local = false;
                vec![Effect::Run(Run::Ordinary)]
            }
            Message::Finished(finished) => self.finished(finished),
        }
    }
    fn finished(&mut self, finished: Finished) -> Vec<Effect> {
        let consent = std::mem::take(&mut self.consent_run);
        match finished {
            Finished::Outcome(outcome) => {
                self.model.outcome(&outcome);
                match outcome {
                    Outcome::WaitingForElectron => {
                        if let Some(config) = &self.plan.config {
                            self.model.electron_owner_local =
                                electron_owner(&config.electron_profile) == ElectronOwner::Local;
                        }
                        self.model.outcome(&Outcome::WaitingForElectron);
                        if self.model.electron_owner_local {
                            vec![Effect::Watch]
                        } else {
                            vec![]
                        }
                    }
                    Outcome::NeedsSecretConsent { .. }
                    | Outcome::NeedsFolderAccess { .. }
                    | Outcome::Complete { .. } => vec![],
                }
            }
            Finished::Ready(outcome, report) => {
                self.model.outcome(&outcome);
                // An unread report may hide conflicts, so it can't count as clean.
                let reported = match &report {
                    Ok(report) => {
                        self.model.report(report);
                        report.rows.is_empty()
                    }
                    Err(_) => {
                        self.report_unread = true;
                        self.model.report_needs_review = true;
                        false
                    }
                };
                let clean = reported
                    && self
                        .model
                        .summary
                        .as_ref()
                        .is_none_or(|s| s.rows().is_empty());
                if clean {
                    self.after_migration()
                } else {
                    vec![]
                }
            }
            Finished::Paused => {
                self.model.gate = Gate::Paused;
                vec![]
            }
            Finished::LowDisk(low) => {
                self.low_disk = Some((bytes(low.needed), bytes(low.available)));
                self.model.gate = Gate::LowDisk;
                vec![]
            }
            Finished::ProfileRefused(details) => {
                self.details = Some(details);
                self.model.gate = Gate::ProfileRefused;
                vec![]
            }
            Finished::Denied => {
                self.model.gate = Gate::Denied;
                vec![]
            }
            Finished::Failed(failure, details) => {
                self.details = Some(details);
                self.model.failure = failure;
                self.model.gate = if consent { Gate::Denied } else { Gate::Failed };
                vec![]
            }
        }
    }
    fn onboarding_action(&mut self, id: &str) -> Vec<Effect> {
        let Some(onboarding) = self.onboarding.as_mut() else {
            return vec![];
        };
        if let Some(choice) = id.strip_prefix("choose:") {
            match onboarding.step {
                Onboard::Timeline => self.choices.timeline = choice.into(),
                Onboard::Agent => self.choices.agent = choice.into(),
                _ => {}
            }
            return vec![];
        }
        match id {
            "begin" => {
                onboarding.welcome();
                let _ = onboarding.advance();
            }
            "continue" => {
                let _ = onboarding.advance();
            }
            "back" => {
                let _ = onboarding.back();
                if onboarding.step == Onboard::Welcome {
                    onboarding.step = Onboard::Animation;
                }
            }
            "import" | "fresh" => {
                let mut raw = serde_json::json!({
                    "timelineMode": self.choices.timeline,
                    "workspaceImport": serde_json::Value::Null,
                });
                raw["agent"] = serde_json::json!({
                    "provider": self.choices.agent,
                    "model": match self.choices.agent.as_str() {
                        "claude" => "claude-opus",
                        "compatible" => "compatible",
                        _ => "gpt-5",
                    },
                });
                if id == "import" {
                    raw["workspaceImport"] = "after-effects".into();
                }
                match Chosen::parse(raw.clone()).and_then(|choice| onboarding.begin(choice)) {
                    Ok(_) => {
                        self.chosen = Some(raw);
                        return self.enter();
                    }
                    Err(_) => {
                        self.error =
                            Some("Couldn’t open your workspace. Try again, or start fresh.".into())
                    }
                }
            }
            "quit" => return vec![Effect::Quit],
            _ => {}
        }
        vec![]
    }
    pub fn action(&mut self, id: &str) -> Vec<Effect> {
        self.error = None;
        match (&self.entry, id) {
            (Entry::Waiting, _) => {}
            (Entry::Entering { .. } | Entry::Failed { .. }, "quit") => return vec![Effect::Quit],
            (Entry::Failed { .. }, "try_again") => return self.enter(),
            // Already on its way: a second click or a held Return does nothing.
            _ => return vec![],
        }
        if self.onboarding.is_some() {
            return self.onboarding_action(id);
        }
        match id {
            "quit" => vec![Effect::Quit],
            "pause" => {
                self.model.pause_at_boundary();
                if self.model.gate == Gate::Pausing {
                    vec![Effect::Pause]
                } else {
                    vec![]
                }
            }
            // Only once the worker has stopped: a second one would lose the lock.
            "resume" if self.model.gate == Gate::Paused => {
                self.model.resume();
                vec![Effect::Run(Run::Ordinary)]
            }
            "check_again" | "try_again" => {
                self.model.gate = Gate::Resuming;
                vec![Effect::Run(Run::Ordinary)]
            }
            "resume_with_secret_consent" => {
                self.model.gate = Gate::WaitingForMacos;
                self.consent_run = true;
                vec![Effect::Run(Run::WithConsent)]
            }
            "skip_secret_consent" => {
                self.model.gate = Gate::Resuming;
                vec![Effect::Run(Run::SkipConsent)]
            }
            "allow_folders" | "skip_folders" if self.model.gate == Gate::FolderAccess => {
                self.model.gate = Gate::Resuming;
                vec![Effect::Run(Run::Folders(if id == "allow_folders" {
                    FolderChoice::Allow
                } else {
                    FolderChoice::Skip
                }))]
            }
            "skip_folder" if self.model.waiting_folder.is_some() => vec![Effect::SkipFolder],
            "show_report" => vec![Effect::ShowReport],
            "open_native" => self.after_migration(),
            _ => vec![],
        }
    }
    fn migration_frame(&self) -> Frame {
        let view = match (&self.low_disk, self.model.gate) {
            (Some((needed, available)), Gate::LowDisk) => {
                self.model.low_disk_view(needed, available)
            }
            _ => self.model.view(),
        };
        let paused = self.model.gate == Gate::Paused;
        let steps = self
            .model
            .rows()
            .into_iter()
            .map(|row| StepRow {
                title: row.title,
                value: row.value,
                progress: row.active.map(|fraction| match fraction {
                    Some(fraction) => Progress::Determinate(fraction),
                    // A paused step with no total doesn't slide.
                    None if paused => Progress::Determinate(0.),
                    None => Progress::Indeterminate,
                }),
            })
            .collect();
        let unread = self.report_unread.then(|| Row {
            symbol: "exclamationmark.triangle.fill",
            severity: Severity::Warning,
            text: "The Migration Report couldn’t be read, so this list may be incomplete.".into(),
        });
        let rows = self.model.summary.as_ref().map_or(vec![], |summary| {
            summary
                .rows()
                .into_iter()
                .map(|(id, text)| {
                    let (symbol, severity) = match id {
                        // An empty list entry lost nothing: a note, not a warning.
                        "history_refusals" => ("exclamationmark.triangle.fill", Severity::Warning),
                        "blocked_owners" => ("xmark.octagon.fill", Severity::Danger),
                        _ => ("info.circle", Severity::Note),
                    };
                    Row {
                        symbol,
                        severity,
                        text,
                    }
                })
                .chain(unread)
                .collect()
        });
        Frame {
            surface: view.surface.into(),
            title: view.title,
            line: view.line,
            spinner: matches!(
                self.model.gate,
                Gate::WaitingLocal | Gate::WaitingForMacos | Gate::Resuming
            ) || self.model.waiting_folder.is_some(),
            status: view.status.filter(|s| !s.is_empty()),
            steps,
            rows,
            details: self
                .details
                .clone()
                .filter(|_| self.model.details_available()),
            buttons: view
                .buttons
                .iter()
                .map(|b| Button {
                    id: b.id.into(),
                    label: b.label.into(),
                    enabled: b.enabled,
                    leading: b.id == "quit",
                })
                .collect(),
            default: view.default.map(str::to_owned),
            ..Frame::default()
        }
    }
    fn onboarding_frame(&self, onboarding: &Onboarding) -> Frame {
        let choice = |id: &str, label: &str, detail: Option<&str>, lines: &[&str]| Choice {
            id: id.into(),
            label: label.into(),
            detail: detail.map(str::to_owned),
            lines: lines.iter().map(|s| (*s).to_owned()).collect(),
        };
        let continuing = |frame: Frame| Frame {
            buttons: vec![Button::new("continue", "Continue")],
            default: Some("continue".into()),
            ..frame
        };
        match onboarding.step {
            Onboard::Animation | Onboard::Welcome => Frame {
                surface: "S107".into(),
                animate: true,
                // Two lines, as v1.2.0 (read as one sentence).
                title: "Welcome to\nPowermove.".into(),
                centred: true,
                buttons: vec![Button::new("begin", "Begin")],
                default: Some("begin".into()),
                ..Frame::default()
            },
            Onboard::Timeline => continuing(Frame {
                surface: "S108".into(),
                back: Some("Back to Welcome".into()),
                title: "Pick Your Timeline.".into(),
                line: "Work the way you already do. You can switch anytime from the timeline panel."
                    .into(),
                cards: true,
                choices: vec![
                    choice(
                        "layers",
                        "Layers",
                        None,
                        &["1  Title", "2  Lower third", "3  B-roll", "4  Interview", "5  Music"],
                    ),
                    choice(
                        "tracks",
                        "Tracks",
                        None,
                        &["V2  Title, Lower third", "V1  Interview, B-roll", "A1  Music"],
                    ),
                ],
                chosen: Some(self.choices.timeline.clone()),
                ..Frame::default()
            }),
            Onboard::Agent => continuing(Frame {
                surface: "S109".into(),
                back: Some("Back to Timeline Choice".into()),
                title: "Pick Your Agent.".into(),
                line: "It builds what you ask for, right in the editor. You can switch anytime from the agent’s composer."
                    .into(),
                choices: vec![
                    choice("chatgpt", "ChatGPT", Some("Uses your ChatGPT plan through Codex."), &[]),
                    choice("claude", "Claude", Some("Uses your Claude plan through Claude Code."), &[]),
                    choice(
                        "compatible",
                        "API or Local Model",
                        Some("Any OpenAI-compatible API, Ollama or LM Studio. Connect it in Settings."),
                        &[],
                    ),
                ],
                chosen: Some(self.choices.agent.clone()),
                ..Frame::default()
            }),
            Onboard::Transcription => continuing(Frame {
                surface: "S109".into(),
                back: Some("Back to Agent Choice".into()),
                title: "Pick Your Speech Model.".into(),
                line: "Captions and your agent can transcribe speech on this Mac. It’s optional, and the download runs while you work."
                    .into(),
                // Models download from Settings › Transcription (S092) and the
                // Generate Captions sheet (S091); S109's in-flow chooser isn't built.
                status: Some("Speech models can be downloaded later from Settings.".into()),
                ..Frame::default()
            }),
            Onboard::Workspace => Frame {
                surface: "S110".into(),
                back: Some("Back to Speech Model Choice".into()),
                eyebrow: Some("After Effects".into()),
                title: "Grab Your Suitcase.".into(),
                line: "Bring your panels and tools along. Your agent will unpack them in Powermove."
                    .into(),
                error: self.error.clone(),
                buttons: vec![
                    Button::new("fresh", "Start Fresh"),
                    Button::new("import", "Bring My Workspace"),
                ],
                default: Some("import".into()),
                ..Frame::default()
            },
            Onboard::Starting | Onboard::Complete => Frame {
                surface: "S110".into(),
                title: "Grab Your Suitcase.".into(),
                status: Some("Opening Powermove…".into()),
                spinner: true,
                ..Frame::default()
            },
        }
    }
    pub fn frame(&self) -> Frame {
        match (&self.entry, &self.onboarding) {
            (Entry::Failed { details }, _) => library_failed_frame(details),
            (Entry::Entering { updating: true }, _) => updating_frame(),
            (Entry::Entering { updating: false }, None) => Frame {
                surface: "N003".into(),
                title: "Your Library Is Ready".into(),
                status: Some("Opening Powermove…".into()),
                spinner: true,
                ..Frame::default()
            },
            (_, Some(onboarding)) => self.onboarding_frame(onboarding),
            (Entry::Waiting, None) => self.migration_frame(),
        }
    }
}

fn library_failed_frame(details: &str) -> Frame {
    Frame {
        surface: "N001".into(),
        title: "Your Library Couldn’t Open".into(),
        line: "Nothing was changed. If Powermove is open in another window or on another account, quit it, then try again.".into(),
        details: Some(details.to_owned()),
        buttons: vec![
            Button::new("quit", "Quit Powermove").leading(),
            Button::new("try_again", "Try Again"),
        ],
        default: Some("try_again".into()),
        ..Frame::default()
    }
}
fn updating_frame() -> Frame {
    Frame {
        surface: "N001".into(),
        title: "Updating Your Library".into(),
        line: "Your original app stays unchanged.".into(),
        status: Some("Bringing in your recent work…".into()),
        spinner: true,
        buttons: vec![Button::new("quit", "Quit Powermove").leading()],
        ..Frame::default()
    }
}
/// Runs the coordinator on a worker, and reads the report there too: it walks the
/// store and replays journals. Pausing lands on the next durable boundary.
pub fn spawn_run(
    config: Config,
    run: Run,
    pause: Arc<AtomicBool>,
    activity: Arc<Activity>,
    tx: async_channel::Sender<Message>,
) {
    let native = config.native_profile.clone();
    std::thread::spawn(move || {
        // Originals in folders macOS protects are opened only after the person
        // saw the explanation, and never waited on past an unanswered prompt.
        let _folders = source_access::install(FolderChoice::Ask, activity);
        let events = tx.clone();
        let stop = pause.clone();
        let hook = move |event: Event| -> Result<()> {
            let _ = events.send_blocking(Message::Event(event));
            anyhow::ensure!(!stop.load(Ordering::Acquire), PauseRequested);
            Ok(())
        };
        let result = match run {
            Run::Ordinary => pm_first_launch::run(&config, hook),
            Run::SkipConsent => pm_first_launch::skip_secret_consent(&config, hook),
            Run::Folders(choice) => pm_first_launch::run_with_folder_access(&config, choice, hook),
            // Constructed only here, after the person's click for this request.
            Run::WithConsent => pm_first_launch::resume_with_secret_consent(
                &config,
                &mut MacKeychain::after_user_consent(),
                hook,
            ),
        };
        let finished = match classify(result) {
            Finished::Outcome(outcome @ Outcome::Complete { .. }) => {
                let report = pm_first_launch::report::load(&native).map_err(|e| format!("{e:#}"));
                Finished::Ready(outcome, report)
            }
            finished => finished,
        };
        // `PM_NATIVE_MAIN_TRACE`: how the copy ended, for unattended checks.
        crate::diag::line(serde_json::json!({"migration": format!("{finished:?}")
            .chars()
            .take(300)
            .collect::<String>()}));
        let _ = tx.send_blocking(Message::Finished(finished));
    });
}
#[derive(Debug)]
struct PauseRequested;
impl std::fmt::Display for PauseRequested {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("paused")
    }
}
fn classify(result: Result<Outcome>) -> Finished {
    let error = match result {
        Ok(outcome) => return Finished::Outcome(outcome),
        Err(error) => error,
    };
    if error.to_string() == PauseRequested.to_string() {
        return Finished::Paused;
    }
    if let Some(low) = error.downcast_ref::<LowDisk>() {
        return Finished::LowDisk(*low);
    }
    if error.downcast_ref::<KeychainDenied>().is_some() {
        return Finished::Denied;
    }
    let details = format!("{error:#}");
    if error.downcast_ref::<ProfileRefused>().is_some() {
        return Finished::ProfileRefused(details);
    }
    let permission = error
        .chain()
        .filter_map(|e| e.downcast_ref::<std::io::Error>())
        .any(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
    let failure = if permission {
        Failure::Permission
    } else if error.downcast_ref::<serde_json::Error>().is_some() {
        Failure::InvalidData
    } else {
        Failure::Unknown
    };
    Finished::Failed(failure, details)
}
/// The local original app quit, or its lock turned unreadable. Polled off main.
pub fn spawn_watch(profile: PathBuf, tx: async_channel::Sender<Message>) {
    std::thread::spawn(move || {
        while electron_owner(&profile) == ElectronOwner::Local {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if tx.is_closed() {
                return;
            }
        }
        let _ = tx.send_blocking(Message::ElectronClosed);
    });
}
/// Durable onboarding marker. Written last, so a closed window runs it again.
pub fn write_marker(profile: &Path, choices: &serde_json::Value) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut record = choices.clone();
    record["version"] = 1.into();
    let target = profile.join("onboarding-v1.json");
    let temporary = profile.join(format!("onboarding-v1.json.{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(format!("{record}\n").as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, target)
}
/// A later launch: merge what changed in the original app before the library
/// attaches. The caller logs an error and carries on; the capture resumes next launch.
pub fn later_store(plan: &Plan, events: impl FnMut(Event) -> Result<()>) -> Result<bool> {
    let Some(config) = &plan.config else {
        return Ok(false);
    };
    if plan.launch != Launch::Later || !config.electron_profile.is_dir() {
        return Ok(false);
    }
    Ok(matches!(
        pm_first_launch::delta_if_changed(config, events)?,
        DeltaPrepared::MediaPending
    ))
}

/// One window, one flow. With nothing to show, a later launch merges and opens.
pub fn gate(
    plan: Plan,
    kernel: pm_kernel::KernelHandle,
    tools: crate::mcp::Tools,
    server: Server,
) -> pm_shell::Gate {
    Box::new(move |finish, cx| {
        let (tx, rx) = async_channel::unbounded();
        let driver = Driver {
            library: Library {
                plan: plan.clone(),
                kernel,
                tools,
                server,
                attached: Default::default(),
            },
            finish,
            pause: Default::default(),
            activity: Default::default(),
            running: Default::default(),
            flow: Rc::new(RefCell::new(Flow::new(plan))),
            window: Rc::default(),
            tx,
        };
        driver.begin(rx, cx);
    })
}
pub type Server = Arc<std::sync::Mutex<Option<pm_mcp::transport::LoopbackServer>>>;
#[derive(Clone)]
struct Library {
    plan: Plan,
    kernel: pm_kernel::KernelHandle,
    tools: crate::mcp::Tools,
    server: Server,
    /// A retry after a later step failed must not attach a second time.
    attached: Arc<AtomicBool>,
}
struct Opened {
    /// A pending capture still has media to bring in.
    media: bool,
    /// The person chose Bring My Workspace on the last step.
    bring_workspace: bool,
    /// The merge before the attach stopped.
    later_failed: bool,
}
const LATER_FAILED: &str = "Recent changes from the original app couldn’t be brought in. Powermove will try again next launch.";
#[derive(Clone)]
struct Driver {
    library: Library,
    finish: pm_shell::Finish,
    pause: Arc<AtomicBool>,
    /// The worker's live bytes and folder prompt, polled while it runs.
    activity: Arc<Activity>,
    running: Rc<std::cell::Cell<bool>>,
    flow: Rc<RefCell<Flow>>,
    window: Rc<RefCell<Option<gpui::WindowHandle<pm_shell::welcome_view::WelcomeView>>>>,
    tx: async_channel::Sender<Message>,
}
impl Driver {
    fn begin(self, rx: async_channel::Receiver<Message>, cx: &mut gpui::App) {
        let effects = self.flow.borrow_mut().start();
        self.show(cx);
        cx.spawn({
            let driver = self.clone();
            async move |cx| {
                while let Ok(message) = rx.recv().await {
                    cx.update(|cx| {
                        if let Message::Finished(_) = &message {
                            driver.running.set(false);
                        }
                        if let Message::Finished(Finished::Failed(_, details)) = &message {
                            crate::error_log::shared(cx)
                                .borrow_mut()
                                .push("Library", details);
                        }
                        driver.step(|flow| flow.message(message), cx);
                    });
                }
            }
        })
        .detach();
        self.apply(effects, cx);
    }
    fn step(&self, change: impl FnOnce(&mut Flow) -> Vec<Effect>, cx: &mut gpui::App) {
        let effects = change(&mut self.flow.borrow_mut());
        self.show(cx);
        self.apply(effects, cx);
    }
    /// Puts the flow's frame on screen. The first frame that needs a window opens it.
    fn show(&self, cx: &mut gpui::App) {
        let (frame, visible) = {
            let flow = self.flow.borrow();
            (flow.frame(), flow.visible())
        };
        if !frame.animate {
            stop_chime();
        }
        let handle = *self.window.borrow();
        let animating = frame.animate;
        let began = match handle {
            Some(handle) => handle
                .update(cx, |view, _, cx| {
                    let began = animating && !view.frame.animate;
                    view.set_frame(frame, cx);
                    began
                })
                .unwrap_or(false),
            None if visible => {
                let on_action: pm_shell::welcome_view::OnAction = Rc::new({
                    let driver = self.clone();
                    move |id, cx| driver.step(|flow| flow.action(id), cx)
                });
                let opened =
                    pm_shell::welcome_view::open_welcome(pm_shell::tokens(), frame, on_action, cx);
                match opened {
                    Ok(handle) => {
                        *self.window.borrow_mut() = Some(handle);
                        // A development review walks to a step: the steps it
                        // names (`--review-onboarding begin,continue`), never
                        // the last one, which finishes and writes.
                        #[cfg(debug_assertions)]
                        if let Ok(steps) = std::env::var("PM_NATIVE_REVIEW_ONBOARDING") {
                            let ids: Vec<String> = steps
                                .split(',')
                                .filter(|s| {
                                    matches!(*s, "begin" | "continue" | "back")
                                        || s.starts_with("choose:")
                                })
                                .map(str::to_owned)
                                .collect();
                            let driver = self.clone();
                            cx.spawn(async move |cx| {
                                cx.background_executor()
                                    .timer(std::time::Duration::from_secs(1))
                                    .await;
                                let _ = cx.update(|cx| {
                                    for id in ids {
                                        driver.step(|flow| flow.action(&id), cx);
                                    }
                                });
                            })
                            .detach();
                        }
                        animating
                    }
                    Err(error) => {
                        eprintln!("Could not open Welcome: {error:#}");
                        false
                    }
                }
            }
            None => false,
        };
        if began {
            play_chime();
        }
    }
    fn apply(&self, effects: Vec<Effect>, cx: &mut gpui::App) {
        for effect in effects {
            match effect {
                Effect::Run(run) => {
                    self.pause.store(false, Ordering::Release);
                    if let Some(config) = self.library.plan.config.clone() {
                        spawn_run(
                            config,
                            run,
                            self.pause.clone(),
                            self.activity.clone(),
                            self.tx.clone(),
                        );
                        self.poll(cx);
                    }
                }
                Effect::SkipFolder => self.activity.skip_waiting(),
                Effect::Pause => self.pause.store(true, Ordering::Release),
                Effect::Watch => {
                    if let Some(config) = &self.library.plan.config {
                        spawn_watch(config.electron_profile.clone(), self.tx.clone());
                    }
                }
                Effect::Quit => cx.quit(),
                Effect::ShowReport => {
                    crate::migration_report::open(self.library.plan.profile.clone(), cx)
                }
                Effect::Enter(choices) => self.enter(choices, cx),
            }
        }
    }
    /// Four times a second while the worker runs: its bytes and any folder prompt.
    fn poll(&self, cx: &mut gpui::App) {
        if self.running.replace(true) {
            return;
        }
        let driver = self.clone();
        cx.spawn(async move |cx| {
            while driver.running.get() {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let (bytes, waiting) = (driver.activity.bytes(), driver.activity.waiting());
                cx.update(|cx| {
                    let changed = driver.flow.borrow_mut().activity(bytes, waiting);
                    if changed {
                        driver.show(cx);
                    }
                });
            }
        })
        .detach();
    }
    /// Library first, then the MCP server, then windows. Off main until the last.
    fn enter(&self, choices: Option<serde_json::Value>, cx: &mut gpui::App) {
        let library = self.library.clone();
        let work = cx
            .background_executor()
            .spawn(async move { library.open(choices) });
        // Merging later changes normally finishes unseen. If it doesn't, say so.
        let done = Rc::new(std::cell::Cell::new(false));
        let late = self.window.borrow().is_none().then(|| {
            let (done, driver) = (done.clone(), self.clone());
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                cx.update(|cx| {
                    if !done.get() {
                        driver.step(
                            |flow| {
                                flow.updating();
                                vec![]
                            },
                            cx,
                        );
                    }
                });
            })
        });
        let driver = self.clone();
        cx.spawn(async move |cx| {
            let result = work.await;
            done.set(true);
            drop(late);
            cx.update(|cx| match result {
                Ok(opened) => driver.opened(opened, cx),
                Err(error) => {
                    let details = format!("{error:#}");
                    crate::error_log::shared(cx)
                        .borrow_mut()
                        .push("Library", &details);
                    driver.step(|flow| flow.enter_failed(details), cx);
                }
            });
        })
        .detach();
    }
    fn opened(&self, opened: Opened, cx: &mut gpui::App) {
        let log = crate::error_log::shared(cx);
        if opened.later_failed {
            log.borrow_mut().tell("Library", LATER_FAILED);
        }
        // The editor that opens next starts the transfer (`firstRunWorkspace`).
        note_workspace_request(opened.bring_workspace);
        (self.finish)(cx);
        if let Some(handle) = self.window.borrow_mut().take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        if opened.media {
            self.library.finish_media(log, cx);
        }
    }
}
impl Library {
    fn open(&self, choices: Option<serde_json::Value>) -> Result<Opened> {
        let later = later_store(&self.plan, |_| Ok(()));
        if let Err(error) = &later {
            eprintln!("Bringing in later changes stopped: {error:#}");
        }
        if !self.attached.load(Ordering::Acquire) {
            crate::mcp::attach(&self.kernel, &self.plan.profile)?;
            self.attached.store(true, Ordering::Release);
        }
        if self.server.lock().unwrap().is_none() {
            let tools = self.tools.clone();
            let server =
                pm_mcp::transport::LoopbackServer::start_with(&self.plan.profile, move || {
                    tools.connect()
                })?;
            *self.server.lock().unwrap() = Some(server);
        }
        let choices_bring_workspace = choices.as_ref().is_some_and(requests_workspace);
        if let Some(choices) = choices {
            std::fs::create_dir_all(&self.plan.profile)?;
            if let Some(provider) = choices["agent"]["provider"].as_str() {
                let _pending = self
                    .kernel
                    .state_transaction(pm_kernel::state::StateTransaction {
                        ops: vec![pm_kernel::state::StateOp::Setting {
                            key: "agentProvider".into(),
                            value: Some(provider.into()),
                        }],
                        ..Default::default()
                    });
            }
            write_marker(&self.plan.profile, &choices)?;
        }
        Ok(Opened {
            bring_workspace: choices_bring_workspace,
            media: *later.as_ref().unwrap_or(&false),
            later_failed: later.is_err(),
        })
    }
    /// Lands behind the open library, on its own thread.
    fn finish_media(&self, log: crate::error_log::Shared, cx: &mut gpui::App) {
        let Some(config) = self.plan.config.clone() else {
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = pm_first_launch::delta_media(&config, |_| Ok(()));
            if let Err(error) = &result {
                eprintln!("Bringing in new media stopped: {error:#}");
            }
            let _ = tx.send_blocking(result.is_ok());
        });
        cx.spawn(async move |cx| {
            match rx.recv().await {
                Ok(true) => {
                    // Protected folders it couldn't open show in Settings › Media.
                    cx.update(crate::pro_media::MediaPassDone::bump);
                }
                Ok(false) => log.borrow_mut().tell("Library", LATER_FAILED),
                Err(_) => {}
            }
        })
        .detach();
    }
}
/// The choices ask to bring the After Effects workspace (v1's
/// `workspaceImport`).
pub fn requests_workspace(choices: &serde_json::Value) -> bool {
    choices["workspaceImport"] == "after-effects"
}
thread_local! {
    /// Bring My Workspace was chosen and no editor has taken it yet.
    static WORKSPACE_REQUEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// The library opened after onboarding; `on`: Bring My Workspace was chosen.
pub(crate) fn note_workspace_request(on: bool) {
    WORKSPACE_REQUEST.with(|r| r.set(on));
}
/// The first-run request for the workspace transfer, taken once.
pub fn take_workspace_request() -> bool {
    WORKSPACE_REQUEST.with(|r| r.replace(false))
}
thread_local! {
    static CHIME: RefCell<Option<objc2::rc::Retained<objc2_app_kit::NSSound>>> =
        const { RefCell::new(None) };
}
/// The original scene's sound, once, with the animation. Reduce Motion is silent.
fn play_chime() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSSound, NSWorkspace};
    use objc2_foundation::NSData;
    const SOUND: &[u8] = include_bytes!("../assets/onboarding-chime.wav");
    if MainThreadMarker::new().is_none()
        || NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    {
        return;
    }
    let sound = NSSound::initWithData(NSSound::alloc(), &NSData::with_bytes(SOUND));
    if let Some(sound) = sound {
        sound.play();
        CHIME.with(|slot| *slot.borrow_mut() = Some(sound));
    }
}
fn stop_chime() {
    CHIME.with(|slot| {
        if let Some(sound) = slot.borrow_mut().take() {
            sound.stop();
        }
    });
}
#[cfg(test)]
#[path = "first_run_tests.rs"]
mod tests;
