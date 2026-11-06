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
    surfaces::migration::{Failure, Gate, Model},
};
use pm_kernel::locks::{ElectronOwner, electron_owner};
use pm_services::surface_backing::onboarding::{Choice as Chosen, Onboarding, Step as Onboard};
use pm_shell::welcome::{Button, Choice, Frame, Progress, Row, Severity};
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
        Some(Self {
            onboard: !isolated && !onboarding_completed(&profile),
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
}
#[derive(Clone, Debug)]
pub enum Finished {
    Outcome(Outcome),
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
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Run(Run),
    Pause,
    /// Watch the local original app until it quits.
    Watch,
    Quit,
    ShowReport,
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
    report_rows: usize,
    error: Option<String>,
    consent_run: bool,
}
fn bytes(value: u64) -> String {
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
            report_rows: 0,
            error: None,
            consent_run: false,
            plan,
        }
    }
    /// Where the window starts. `None` means no Welcome window at all.
    pub fn start(&mut self) -> Option<Effect> {
        if self.plan.launch == Launch::Migrate {
            return Some(Effect::Run(Run::Ordinary));
        }
        if self.plan.onboard {
            self.onboarding = Some(Onboarding::new(true));
        }
        None
    }
    pub fn visible(&self) -> bool {
        self.plan.launch == Launch::Migrate || self.onboarding.is_some()
    }
    fn after_migration(&mut self) -> Vec<Effect> {
        if self.plan.onboard && !onboarding_completed(&self.plan.profile) {
            self.onboarding = Some(Onboarding::new(true));
            return vec![];
        }
        vec![Effect::Enter(None)]
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
                    Outcome::NeedsSecretConsent { .. } => vec![],
                    Outcome::Complete { .. } => {
                        if let Ok(report) = pm_first_launch::report::load(&self.plan.profile) {
                            self.report_rows = report.rows.len();
                            self.model.report(&report);
                        }
                        let clean = self.report_rows == 0
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
                    Ok(_) => return vec![Effect::Enter(Some(raw))],
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
        if self.onboarding.is_some() {
            return self.onboarding_action(id);
        }
        match id {
            "quit" => vec![Effect::Quit],
            "pause" => {
                self.model.pause_at_boundary();
                vec![Effect::Pause]
            }
            "resume" => {
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
        let progress = (self.model.gate == Gate::Copying || self.model.gate == Gate::Paused)
            .then(|| self.model.progress.as_ref())
            .flatten()
            .map(|p| {
                p.determinate()
                    .map_or(Progress::Indeterminate, |(done, total)| {
                        Progress::Determinate { done, total }
                    })
            });
        let rows = self.model.summary.as_ref().map_or(vec![], |summary| {
            summary
                .rows()
                .into_iter()
                .map(|(id, text)| {
                    let (symbol, severity) = match id {
                        "history_refusals" | "missing_projects" => {
                            ("exclamationmark.triangle.fill", Severity::Warning)
                        }
                        "blocked_owners" => ("xmark.octagon.fill", Severity::Danger),
                        _ => ("info.circle", Severity::Note),
                    };
                    Row {
                        symbol,
                        severity,
                        text,
                    }
                })
                .collect()
        });
        Frame {
            surface: view.surface.into(),
            title: view.title,
            line: view.line,
            spinner: matches!(
                self.model.gate,
                Gate::WaitingLocal | Gate::WaitingForMacos | Gate::Resuming
            ),
            status: view.status.filter(|s| !s.is_empty()),
            progress,
            rows,
            details: self
                .model
                .details_available()
                .then(|| self.details.clone())
                .flatten(),
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
                title: "Welcome to Powermove.".into(),
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
                // No speech model service in the app yet: the spec's unavailable state.
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
        match &self.onboarding {
            Some(onboarding) => self.onboarding_frame(onboarding),
            None => self.migration_frame(),
        }
    }
}

fn updating_frame() -> Frame {
    Frame {
        surface: "N001".into(),
        title: "Updating Your Library".into(),
        line: "Your original app stays unchanged.".into(),
        status: Some("Bringing in your recent work…".into()),
        spinner: true,
        buttons: vec![Button::new("quit", "Quit Powermove Native").leading()],
        ..Frame::default()
    }
}
/// Runs the coordinator on a worker. Pausing lands on the next durable boundary.
pub fn spawn_run(
    config: Config,
    run: Run,
    pause: Arc<AtomicBool>,
    tx: async_channel::Sender<Message>,
) {
    std::thread::spawn(move || {
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
            // Constructed only here, after the person's click for this request.
            Run::WithConsent => pm_first_launch::resume_with_secret_consent(
                &config,
                &mut MacKeychain::after_user_consent(),
                hook,
            ),
        };
        let _ = tx.send_blocking(Message::Finished(classify(result)));
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
/// attaches. An error never stops the app; the capture resumes next launch.
pub fn later_store(plan: &Plan, events: impl FnMut(Event) -> Result<()>) -> bool {
    let Some(config) = &plan.config else {
        return false;
    };
    if plan.launch != Launch::Later || !config.electron_profile.is_dir() {
        return false;
    }
    match pm_first_launch::delta_if_changed(config, events) {
        Ok(DeltaPrepared::MediaPending) => true,
        Ok(_) => false,
        Err(error) => {
            eprintln!("Bringing in later changes stopped: {error:#}");
            false
        }
    }
}

/// One window, one flow. With nothing to show, a later launch merges and opens.
pub fn gate(
    plan: Plan,
    kernel: pm_kernel::KernelHandle,
    mcp_host: Arc<crate::mcp::Host>,
    server: Server,
) -> Box<dyn FnOnce(pm_shell::Finish, &mut gpui::App)> {
    Box::new(move |finish, cx| {
        let mut flow = Flow::new(plan.clone());
        let first = flow.start();
        let driver = Driver {
            library: Library {
                plan,
                kernel,
                mcp_host,
                server,
            },
            finish,
            pause: Default::default(),
        };
        driver.begin(flow, first, cx);
    })
}
pub type Server = Arc<std::sync::Mutex<Option<pm_mcp::transport::LoopbackServer>>>;
#[derive(Clone)]
struct Library {
    plan: Plan,
    kernel: pm_kernel::KernelHandle,
    mcp_host: Arc<crate::mcp::Host>,
    server: Server,
}
#[derive(Clone)]
struct Driver {
    library: Library,
    finish: pm_shell::Finish,
    pause: Arc<AtomicBool>,
}
type Window = Rc<RefCell<Option<gpui::WindowHandle<pm_shell::welcome_view::WelcomeView>>>>;
impl Driver {
    fn begin(self, flow: Flow, first: Option<Effect>, cx: &mut gpui::App) {
        let (tx, rx) = async_channel::unbounded();
        let window: Window = Rc::default();
        let flow = Rc::new(RefCell::new(flow));
        let visible = flow.borrow().visible();
        if visible {
            let on_action: pm_shell::welcome_view::OnAction = Rc::new({
                let (driver, flow, window, tx) =
                    (self.clone(), flow.clone(), window.clone(), tx.clone());
                move |id, cx| {
                    let effects = flow.borrow_mut().action(id);
                    let frame = flow.borrow().frame();
                    show(&window, frame, cx);
                    driver.apply(effects, &window, &tx, cx);
                }
            });
            let frame = flow.borrow().frame();
            match pm_shell::welcome_view::open_welcome(pm_shell::tokens(), frame, on_action, cx) {
                Ok(handle) => *window.borrow_mut() = Some(handle),
                Err(error) => eprintln!("Could not open Welcome: {error:#}"),
            }
        }
        cx.spawn({
            let (driver, flow, window, tx) = (self.clone(), flow, window.clone(), tx.clone());
            async move |cx| {
                while let Ok(message) = rx.recv().await {
                    let _ = cx.update(|cx| {
                        let effects = flow.borrow_mut().message(message);
                        let frame = flow.borrow().frame();
                        show(&window, frame, cx);
                        driver.apply(effects, &window, &tx, cx);
                    });
                }
            }
        })
        .detach();
        let effects = first.into_iter().collect::<Vec<_>>();
        if visible {
            self.apply(effects, &window, &tx, cx);
        } else {
            self.enter(None, &window, cx);
        }
    }
    fn apply(
        &self,
        effects: Vec<Effect>,
        window: &Window,
        tx: &async_channel::Sender<Message>,
        cx: &mut gpui::App,
    ) {
        for effect in effects {
            match effect {
                Effect::Run(run) => {
                    self.pause.store(false, Ordering::Release);
                    if let Some(config) = self.library.plan.config.clone() {
                        spawn_run(config, run, self.pause.clone(), tx.clone());
                    }
                }
                Effect::Pause => self.pause.store(true, Ordering::Release),
                Effect::Watch => {
                    if let Some(config) = &self.library.plan.config {
                        spawn_watch(config.electron_profile.clone(), tx.clone());
                    }
                }
                Effect::Quit => cx.quit(),
                Effect::ShowReport => {
                    crate::migration_report::open(self.library.plan.profile.clone(), cx)
                }
                Effect::Enter(choices) => self.enter(choices, window, cx),
            }
        }
    }
    /// Library first, then the MCP server, then windows. Off main until the last.
    fn enter(&self, choices: Option<serde_json::Value>, window: &Window, cx: &mut gpui::App) {
        let (driver, window) = (self.clone(), window.clone());
        let library = self.library.clone();
        let work = cx
            .background_executor()
            .spawn(async move { library.open(choices) });
        // Merging later changes normally finishes unseen. If it doesn't, say so.
        let done = Rc::new(std::cell::Cell::new(false));
        let late = window.borrow().is_none().then(|| {
            let (done, window) = (done.clone(), window.clone());
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let _ = cx.update(|cx| {
                    if !done.get() {
                        let on_action = Rc::new(|_: &str, cx: &mut gpui::App| cx.quit());
                        let frame = updating_frame();
                        *window.borrow_mut() = pm_shell::welcome_view::open_welcome(
                            pm_shell::tokens(),
                            frame,
                            on_action,
                            cx,
                        )
                        .ok();
                    }
                });
            })
        });
        cx.spawn(async move |cx| {
            let result = work.await;
            done.set(true);
            drop(late);
            let media = *result.as_ref().unwrap_or(&false);
            let _ = cx.update(|cx| {
                if let Err(error) = &result {
                    eprintln!("Could not open the library: {error:#}");
                }
                (driver.finish)(cx);
                if let Some(handle) = window.borrow_mut().take() {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            });
            if media {
                driver.library.finish_media();
            }
        })
        .detach();
    }
}
impl Library {
    /// Returns whether a pending capture still has media to bring in.
    fn open(&self, choices: Option<serde_json::Value>) -> Result<bool> {
        let media = later_store(&self.plan, |_| Ok(()));
        crate::mcp::attach(&self.kernel, &self.plan.profile)?;
        let host = self.mcp_host.clone();
        let server =
            pm_mcp::transport::LoopbackServer::start_with(&self.plan.profile, move || {
                crate::mcp::Connection::new(host.clone())
            })?;
        *self.server.lock().unwrap() = Some(server);
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
        Ok(media)
    }
    fn finish_media(&self) {
        let Some(config) = self.plan.config.clone() else {
            return;
        };
        std::thread::spawn(move || {
            if let Err(error) = pm_first_launch::delta_media(&config, |_| Ok(())) {
                eprintln!("Bringing in new media stopped: {error:#}");
            }
        });
    }
}
fn show(window: &Window, frame: Frame, cx: &mut gpui::App) {
    let handle = *window.borrow();
    if let Some(handle) = handle {
        let _ = handle.update(cx, |view, _, cx| view.set_frame(frame, cx));
    }
}

#[cfg(test)]
#[path = "first_run_tests.rs"]
mod tests;
