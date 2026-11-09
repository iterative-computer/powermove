//! ID-driven reducer and nonblocking host/service seams.
use super::{form::*, model::*};
use pm_services::ServiceErrorCode;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Load(Query),
    Open(String),
    Create(NewProject),
    Rename { id: String, name: String },
    Duplicate { id: String, name: String },
    Trash(String),
    PutBack(String),
    Archive(String),
}
#[derive(Clone, Debug)]
pub struct Request {
    pub serial: u64,
    pub command: Command,
}
#[derive(Clone, Debug)]
pub enum Outcome {
    Loaded { snapshot: Snapshot, rows: Vec<Row> },
    Changed,
    Duplicated(String),
    Opened(pm_kernel::session::ShellReply),
    Focus(String),
}
#[derive(Clone, Debug)]
pub struct Completion {
    pub serial: u64,
    pub result: Result<Outcome, Failure>,
}

/// enqueue must return immediately. Implementations perform no IO here.
pub trait LibraryPort {
    fn enqueue(&mut self, request: Request) -> Result<(), Failure>;
    fn poll(&mut self) -> Option<Completion>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Projects,
    Empty(EmptyState),
    Loading,
    Unavailable {
        message: &'static str,
        action: &'static str,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameDraft {
    pub id: String,
    pub original: String,
    pub text: String,
}
#[derive(Clone, Debug)]
pub enum HostEffect {
    /// Host must maintain one autosaved, standard titled library window.
    ShowLibrary,
    Rename {
        id: String,
    },
    ShowNewProject,
    CloseNewProject,
    Alert {
        serial: u64,
        alert: Alert,
    },
    Reveal(std::path::PathBuf),
    FocusProject(String),
    OpenEditor {
        reply: pm_kernel::session::ShellReply,
        /// For the alerts that open may need.
        name: String,
        file: Option<std::path::PathBuf>,
    },
    /// Standard medium announcement; errors are high priority.
    Announce(String),
    /// Technical gap only, never displayed as product copy.
    IntegrationGap(&'static str),
}
#[derive(Clone, Debug)]
enum PromptAction {
    Trash(String),
    Archive(String),
    Error,
}
#[derive(Clone, Debug)]
struct Prompt {
    serial: u64,
    alert: Alert,
    action: PromptAction,
}

pub struct LibraryController<P> {
    pub port: P,
    pub query: Query,
    pub form: Option<NewProjectForm>,
    pub rename: Option<RenameDraft>,
    pub snapshot: Snapshot,
    pub rows: Vec<Row>,
    pub selected: Option<String>,
    serial: u64,
    load: Option<u64>,
    loaded: bool,
    failed: bool,
    mutation: Option<Request>,
    pending_duplicate: Option<String>,
    prompts: VecDeque<Prompt>,
    effects: VecDeque<HostEffect>,
}
impl<P: LibraryPort> LibraryController<P> {
    pub fn new(port: P) -> Self {
        Self {
            port,
            // Authored v1.2.0 default (legacy/ui/projects.ts), not a new UX choice.
            query: Query {
                section: Section::Recents,
                ..Default::default()
            },
            form: None,
            rename: None,
            snapshot: Snapshot::default(),
            rows: Vec::new(),
            selected: None,
            serial: 0,
            load: None,
            loaded: false,
            failed: false,
            mutation: None,
            pending_duplicate: None,
            prompts: VecDeque::new(),
            effects: VecDeque::new(),
        }
    }
    fn serial(&mut self) -> u64 {
        self.serial = self
            .serial
            .checked_add(1)
            .expect("library request serial exhausted");
        self.serial
    }
    pub fn show(&mut self) -> Result<(), Failure> {
        self.effects.push_back(HostEffect::ShowLibrary);
        self.refresh()
    }
    pub fn refresh(&mut self) -> Result<(), Failure> {
        let serial = self.serial();
        self.load = Some(serial);
        self.failed = false;
        if let Err(e) = self.port.enqueue(Request {
            serial,
            command: Command::Load(self.query.clone()),
        }) {
            self.load = None;
            self.failed = true;
            return Err(e);
        }
        Ok(())
    }
    pub fn set_query(&mut self, query: Query) -> Result<(), Failure> {
        self.query = query;
        self.refresh()
    }
    pub fn content(&self) -> Content {
        if self.failed {
            Content::Unavailable {
                message: "Your projects can’t be shown right now.",
                action: "Try Again",
            }
        } else if self.load.is_some() && !self.loaded {
            Content::Loading
        } else if self.rows.is_empty() {
            Content::Empty(empty_state(&self.query))
        } else {
            Content::Projects
        }
    }
    /// What the host must do, in order.
    pub fn take_effects(&mut self) -> Vec<HostEffect> {
        self.effects.drain(..).collect()
    }
    pub fn select(&mut self, id: &str) -> Result<(), Failure> {
        if !self.rows.iter().any(|r| r.id == id) {
            return Err(Failure::Stale);
        }
        self.selected = Some(id.into());
        Ok(())
    }
    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() || self.rename.is_some() || self.form.is_some() {
            return;
        }
        let index = self
            .selected
            .as_ref()
            .and_then(|id| self.rows.iter().position(|r| &r.id == id));
        let next = index.map_or(0, |i| {
            i.saturating_add_signed(delta).min(self.rows.len() - 1)
        });
        self.selected = Some(self.rows[next].id.clone());
    }
    /// Escape leaves a rename, or a New Project sheet that isn't submitting.
    /// AppKit owns an alert's keys: nothing reaches the sheet under it.
    pub fn escape(&mut self) {
        if !self.prompts.is_empty() {
            return;
        }
        if self.rename.is_some() {
            self.rename = None;
        } else if self.form.as_ref().is_some_and(|f| !f.submitting) {
            self.form = None;
            self.effects.push_back(HostEffect::CloseNewProject);
        }
    }
    fn row(&self, id: &str) -> Result<&Row, Failure> {
        self.snapshot
            .rows
            .iter()
            .find(|r| r.id == id)
            .ok_or(Failure::Stale)
    }
    pub fn menu(&mut self, id: &str, action: MenuAction) -> Result<(), Failure> {
        if self.mutation.is_some() || self.form.is_some() || self.rename.is_some() {
            return Err(Failure::Busy);
        }
        let row = self.row(id)?.clone();
        if !context_menu(&row).contains(&Some(action)) {
            return Err(Failure::Stale);
        }
        match action {
            MenuAction::Open | MenuAction::OpenNewWindow => self.send(Command::Open(id.into()))?,
            MenuAction::Reveal => self
                .effects
                .push_back(HostEffect::Reveal(row.file.ok_or(Failure::Stale)?)),
            MenuAction::Rename => {
                self.rename = Some(RenameDraft {
                    id: id.into(),
                    original: row.name.clone(),
                    text: row.name,
                });
                self.effects.push_back(HostEffect::Rename { id: id.into() });
            }
            MenuAction::Duplicate => {
                let names = self
                    .snapshot
                    .rows
                    .iter()
                    .map(|r| r.name.clone())
                    .collect::<Vec<_>>();
                self.send(Command::Duplicate {
                    id: id.into(),
                    name: unique(&format!("{} copy", row.name), &names),
                })?;
            }
            MenuAction::Trash => {
                self.prompt(Alert::trash(&row.name), PromptAction::Trash(id.into()))
            }
            MenuAction::PutBack => self.send(Command::PutBack(id.into()))?,
            MenuAction::DeleteImmediately => {
                self.prompt(Alert::delete(&row.name), PromptAction::Archive(id.into()))
            }
        }
        Ok(())
    }
    pub fn double_click(&mut self, id: &str, option: bool) -> Result<(), Failure> {
        self.menu(
            id,
            if option {
                MenuAction::OpenNewWindow
            } else {
                MenuAction::Open
            },
        )
    }
    pub fn commit_rename(&mut self) -> Result<(), Failure> {
        let Some(draft) = self.rename.take() else {
            return Ok(());
        };
        let text = draft.text.trim();
        if text.is_empty() || text == draft.original {
            return Ok(());
        }
        self.row(&draft.id)?;
        self.send(Command::Rename {
            id: draft.id,
            name: text.into(),
        })
    }
    pub fn begin_new(
        &mut self,
        owner: String,
        current: Option<&Format>,
        default: &Format,
    ) -> Result<(), Failure> {
        if self.form.is_some() || self.mutation.is_some() || self.rename.is_some() {
            return Err(Failure::Busy);
        }
        let names = self
            .snapshot
            .rows
            .iter()
            .map(|r| r.name.clone())
            .collect::<Vec<_>>();
        self.form = Some(NewProjectForm::new(owner, current, default, &names));
        self.effects.push_back(HostEffect::ShowNewProject);
        Ok(())
    }
    pub fn create(&mut self) -> Result<(), Failure> {
        let form = self.form.as_ref().ok_or(Failure::Stale)?;
        if !form.can_create() {
            return Err(Failure::Busy);
        }
        let values = form.values().ok_or(Failure::Stale)?;
        self.send(Command::Create(values))?;
        self.form.as_mut().expect("form").submitting = true;
        Ok(())
    }
    fn prompt(&mut self, alert: Alert, action: PromptAction) {
        let serial = self.serial();
        if self.prompts.is_empty() {
            self.effects.push_back(HostEffect::Alert {
                serial,
                alert: alert.clone(),
            });
        }
        self.prompts.push_back(Prompt {
            serial,
            alert,
            action,
        });
    }
    /// Serial and button index are required: delayed callbacks cannot answer a
    /// different alert. Closing/aborting always cancels (None).
    pub fn answer(&mut self, serial: u64, button: Option<usize>) -> Result<(), Failure> {
        let front = self.prompts.front().ok_or(Failure::Stale)?;
        if front.serial != serial {
            return Err(Failure::Stale);
        }
        if button.is_some_and(|i| i >= front.alert.buttons.len()) {
            return Err(Failure::Stale);
        }
        if self.mutation.is_some() && button.is_some_and(|i| front.alert.buttons[i] != "Cancel") {
            return Err(Failure::Busy);
        }
        let prompt = self.prompts.pop_front().expect("front");
        let result = match (prompt.action, button) {
            (PromptAction::Trash(id), Some(0)) => self.send(Command::Trash(id)),
            (PromptAction::Archive(id), Some(1)) => self.send(Command::Archive(id)),
            _ => Ok(()),
        };
        if let Some(next) = self.prompts.front() {
            self.effects.push_back(HostEffect::Alert {
                serial: next.serial,
                alert: next.alert.clone(),
            });
        }
        result
    }
    fn send(&mut self, command: Command) -> Result<(), Failure> {
        if self.mutation.is_some() {
            return Err(Failure::Busy);
        }
        let request = Request {
            serial: self.serial(),
            command,
        };
        self.port.enqueue(request.clone())?;
        self.mutation = Some(request);
        Ok(())
    }
    pub fn poll(&mut self) {
        while let Some(completion) = self.port.poll() {
            if self.load == Some(completion.serial) {
                self.load = None;
                match completion.result {
                    Ok(Outcome::Loaded { snapshot, rows }) => {
                        self.snapshot = snapshot;
                        self.rows = rows;
                        self.loaded = true;
                        self.failed = false;
                        if self
                            .selected
                            .as_ref()
                            .is_some_and(|id| !self.rows.iter().any(|r| &r.id == id))
                        {
                            self.selected = None;
                        }
                        if self.rename.as_ref().is_some_and(|d| {
                            self.snapshot.rows.iter().all(|r| r.id != d.id || r.trashed)
                        }) {
                            self.rename = None;
                        }
                        if let Some(id) = self.pending_duplicate.take() {
                            if self.rows.iter().any(|r| r.id == id) {
                                self.selected = Some(id.clone());
                                if let Ok(row) = self.row(&id) {
                                    self.rename = Some(RenameDraft {
                                        id: id.clone(),
                                        original: row.name.clone(),
                                        text: row.name.clone(),
                                    });
                                }
                                self.effects.push_back(HostEffect::Rename { id });
                            } else {
                                // Opus answer: clear a non-matching search before selecting.
                                // Keep the pending ID through the worker's next load.
                                if self.snapshot.rows.iter().any(|r| {
                                    r.id == id
                                        && !r
                                            .name
                                            .to_lowercase()
                                            .contains(&self.query.text.trim().to_lowercase())
                                }) {
                                    self.query.text.clear();
                                    self.pending_duplicate = Some(id);
                                    let _ = self.refresh();
                                }
                            }
                        }
                    }
                    _ => {
                        self.failed = true;
                        self.effects.push_back(HostEffect::Announce(
                            "Your projects can’t be shown right now.".into(),
                        ));
                    }
                }
                continue;
            }
            if self
                .mutation
                .as_ref()
                .is_none_or(|r| r.serial != completion.serial)
            {
                continue;
            }
            let request = self.mutation.take().expect("matching request");
            match completion.result {
                Ok(outcome) => {
                    match &request.command {
                        Command::Create(_) => {
                            self.form = None;
                            self.effects.push_back(HostEffect::CloseNewProject);
                        }
                        _ => {}
                    }
                    match outcome {
                        Outcome::Duplicated(id) => self.pending_duplicate = Some(id),
                        Outcome::Opened(reply) => {
                            let row = match &request.command {
                                Command::Open(id) => self.rows.iter().find(|r| &r.id == id),
                                _ => None,
                            };
                            let (name, file) = row.map_or_else(
                                || ("Untitled".into(), None),
                                |r| (r.name.clone(), r.file.clone()),
                            );
                            self.effects
                                .push_back(HostEffect::OpenEditor { reply, name, file });
                        }
                        Outcome::Focus(id) => self.effects.push_back(HostEffect::FocusProject(id)),
                        _ => {}
                    }
                    let _ = self.refresh();
                }
                Err(failure) => {
                    if let Some(form) = self.form.as_mut() {
                        form.submitting = false;
                    }
                    if let Failure::Unsupported(gap) = failure {
                        self.effects.push_back(HostEffect::IntegrationGap(gap));
                    }
                    let name = match &request.command {
                        Command::Create(values) => values.name.clone(),
                        Command::Rename { id, .. }
                        | Command::Duplicate { id, .. }
                        | Command::Open(id)
                        | Command::Trash(id)
                        | Command::PutBack(id)
                        | Command::Archive(id) => {
                            self.row(id).map_or(String::new(), |r| r.name.clone())
                        }
                        Command::Load(_) => String::new(),
                    };
                    let code = if let Failure::Service(code) = failure {
                        code
                    } else {
                        ServiceErrorCode::Unavailable
                    };
                    let alert = Alert::error(code, &name);
                    self.effects
                        .push_back(HostEffect::Announce(alert.message.clone()));
                    self.prompt(alert, PromptAction::Error);
                    // A rename may have applied before a subsequent save
                    // failed. Reconcile published dirty state without retrying
                    // the mutation or discarding the user's work.
                    let _ = self.refresh();
                }
            }
        }
    }
}
