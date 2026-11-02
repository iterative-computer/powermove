//! ID-driven reducer and nonblocking host/service seams.
use super::{form::*, model::*};
use std::{collections::VecDeque, ops::Range, time::Duration};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Load(Query),
    Open { id: String, new_window: bool },
    Create(NewProject),
    Rename { id: String, name: String },
    Duplicate { id: String, name: String },
    Trash(String),
    PutBack(String),
    Archive(String),
    PutBackAndSave(String),
}
#[derive(Clone, Debug)]
pub struct Request {
    pub serial: u64,
    pub command: Command,
}
#[derive(Clone, Debug)]
pub enum Outcome {
    Loaded {
        snapshot: Snapshot,
        rows: Vec<Row>,
    },
    Changed,
    Duplicated(String),
    Opened {
        reply: pm_kernel::session::ShellReply,
        new_window: bool,
    },
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
    /// Placeholder wells keep card geometry. Spinner starts only after 0.5 s.
    Loading {
        spinner: bool,
    },
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
#[derive(Clone, Debug, PartialEq)]
pub struct Presentation {
    pub query: Query,
    pub view: View,
    pub content: Content,
    pub selected: Option<String>,
    pub rename: Option<RenameDraft>,
    pub rows: Vec<Row>,
    pub total_rows: usize,
    /// AXList/AXGroup for grid; native table semantics for list.
    pub ax_container: &'static str,
    pub ax_item: &'static str,
    pub searching: bool,
    pub anchor: Option<String>,
    pub form: Option<NewProjectForm>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlertOwner {
    Library,
    Editor(String),
    NewProject(String),
}

#[derive(Clone, Debug)]
pub enum HostEffect {
    /// Host must maintain one autosaved, standard titled library window.
    ShowLibrary,
    FocusSearch,
    Rename {
        id: String,
    },
    ShowNewProject,
    CloseNewProject,
    Alert {
        serial: u64,
        owner: AlertOwner,
        alert: Alert,
    },
    Reveal(std::path::PathBuf),
    FocusProject(String),
    OpenEditor {
        reply: pm_kernel::session::ShellReply,
        new_window: bool,
    },
    SaveAs(String),
    /// Host owns per-window subtitle; no extra copy is generated.
    Subtitle {
        id: String,
        text: Option<&'static str>,
    },
    Banner(&'static str),
    /// Standard medium announcement; errors are high priority.
    Announce {
        text: String,
        high: bool,
    },
    /// Technical gap only, never displayed as product copy.
    IntegrationGap(&'static str),
}
/// The AppKit/GPUI adapter consumes this on its main executor. The renderer
/// owns AppKit focus, keyboard equivalents, sheet placement and AX announcements.
/// It must use NATIVE_PIECES, UI-kit EmptyState and pm-design token values.
pub trait LibraryHost {
    fn effect(&mut self, effect: HostEffect);
    fn publish(&mut self, presentation: Presentation, metrics: WindowMetrics);
}
#[derive(Default)]
pub struct HeadlessHost {
    pub effects: Vec<HostEffect>,
    pub presentations: Vec<(Presentation, WindowMetrics)>,
}
impl LibraryHost for HeadlessHost {
    fn effect(&mut self, effect: HostEffect) {
        self.effects.push(effect);
    }
    fn publish(&mut self, presentation: Presentation, metrics: WindowMetrics) {
        self.presentations.push((presentation, metrics));
    }
}

#[derive(Clone, Debug)]
enum PromptAction {
    Trash(String),
    Archive(String),
    Save(String),
    Error,
}
#[derive(Clone, Debug)]
struct Prompt {
    serial: u64,
    owner: AlertOwner,
    alert: Alert,
    action: PromptAction,
}

pub struct LibraryController<P> {
    pub port: P,
    pub query: Query,
    pub view: View,
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
    undo_trash: Vec<String>,
    pending_duplicate: Option<String>,
    anchor: Option<String>,
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
            view: View::Grid,
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
            undo_trash: Vec::new(),
            pending_duplicate: None,
            anchor: None,
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
    pub fn presentation(&self, elapsed: Duration, range: Range<usize>) -> Presentation {
        let content = if self.failed {
            Content::Unavailable {
                message: "Your projects can’t be shown right now.",
                action: "Try Again",
            }
        } else if self.load.is_some() && !self.loaded {
            Content::Loading {
                spinner: elapsed >= Duration::from_millis(500),
            }
        } else if self.rows.is_empty() {
            Content::Empty(empty_state(&self.query))
        } else {
            Content::Projects
        };
        let end = range.end.min(self.rows.len());
        let start = range.start.min(end);
        Presentation {
            query: self.query.clone(),
            view: self.view,
            content,
            selected: self.selected.clone(),
            rename: self.rename.clone(),
            rows: self.rows[start..end].to_vec(),
            total_rows: self.rows.len(),
            ax_container: if self.view == View::Grid {
                "AXList"
            } else {
                "AXTable"
            },
            ax_item: if self.view == View::Grid {
                "AXGroup"
            } else {
                "AXRow"
            },
            searching: self.load.is_some() && self.loaded,
            anchor: self.anchor.clone(),
            form: self.form.clone(),
        }
    }
    pub fn render(
        &mut self,
        host: &mut impl LibraryHost,
        tokens: &pm_design::Tokens,
        elapsed: Duration,
        range: Range<usize>,
    ) {
        while let Some(effect) = self.effects.pop_front() {
            host.effect(effect);
        }
        host.publish(
            self.presentation(elapsed, range),
            WindowMetrics::resolve(tokens),
        );
    }
    pub fn set_scroll_anchor(&mut self, id: Option<String>) {
        self.anchor = id;
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
    /// Native type-select supplies its accumulated text (system timing/IME).
    pub fn type_select(&mut self, text: &str) {
        if self.rename.is_some() || self.form.is_some() {
            return;
        }
        let prefix = text.to_lowercase();
        if let Some(row) = self
            .rows
            .iter()
            .find(|r| r.name.to_lowercase().starts_with(&prefix))
        {
            self.selected = Some(row.id.clone());
        }
    }
    pub fn key(&mut self, key: Key, columns: usize) -> Result<(), Failure> {
        // AppKit owns the alert's Return/Escape; do not send keystrokes through
        // a modal focus scope to the underlying sheet or collection.
        if !self.prompts.is_empty() {
            return Ok(());
        }
        match key {
            Key::Escape if self.rename.is_some() => {
                self.rename = None;
            }
            Key::Return if self.rename.is_some() => {
                self.commit_rename()?;
            }
            Key::Escape if self.form.as_ref().is_some_and(|f| !f.submitting) => {
                self.form = None;
                self.effects.push_back(HostEffect::CloseNewProject);
            }
            Key::Return if self.form.is_some() => {
                self.create()?;
            }
            _ if self.rename.is_some() || self.form.is_some() => {}
            Key::Find => self.effects.push_back(HostEffect::FocusSearch),
            Key::NewProject => self.effects.push_back(HostEffect::ShowNewProject),
            Key::Left => self.move_selection(-1),
            Key::Right => self.move_selection(1),
            Key::Up => self.move_selection(
                -isize::try_from(if self.view == View::Grid {
                    columns.max(1)
                } else {
                    1
                })
                .unwrap_or(isize::MAX),
            ),
            Key::Down => self.move_selection(
                isize::try_from(if self.view == View::Grid {
                    columns.max(1)
                } else {
                    1
                })
                .unwrap_or(isize::MAX),
            ),
            Key::Return => {
                if let Some(id) = self.selected.clone() {
                    self.menu(&id, MenuAction::Open)?;
                }
            }
            Key::Trash => {
                if let Some(id) = self.selected.clone() {
                    self.menu(&id, MenuAction::Trash)?;
                }
            }
            Key::Undo => self.undo_move_to_trash()?,
            Key::Escape | Key::Other => {}
        }
        Ok(())
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
            MenuAction::Open | MenuAction::OpenNewWindow => self.send(Command::Open {
                id: id.into(),
                new_window: action == MenuAction::OpenNewWindow,
            })?,
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
    /// Host calls on window/project generation invalidation, before publishing
    /// any sheet action. No apply can target a replacement project.
    pub fn owner_changed(&mut self, owner: &str) {
        if self.form.as_ref().is_some_and(|f| f.owner == owner) {
            self.form = None;
            self.effects.push_back(HostEffect::CloseNewProject);
            self.effects.push_back(HostEffect::Banner(
                "The project was changed, so the dialog closed.",
            ));
            self.effects.push_back(HostEffect::Announce {
                text: "The project was changed, so the dialog closed.".into(),
                high: false,
            });
        }
    }
    /// Returns true when the caller must defer normal save to this prompt.
    pub fn before_save(&mut self, id: &str) -> Result<bool, Failure> {
        let row = self.row(id)?.clone();
        if !row.trashed {
            return Ok(false);
        }
        self.prompt(
            Alert::save_in_trash(&row.name),
            PromptAction::Save(id.into()),
        );
        Ok(true)
    }
    fn prompt(&mut self, alert: Alert, action: PromptAction) {
        let serial = self.serial();
        let owner = match &action {
            PromptAction::Save(id) => AlertOwner::Editor(id.clone()),
            PromptAction::Error => self.form.as_ref().map_or(AlertOwner::Library, |f| {
                AlertOwner::NewProject(f.owner.clone())
            }),
            _ => AlertOwner::Library,
        };
        if self.prompts.is_empty() {
            self.effects.push_back(HostEffect::Alert {
                serial,
                owner: owner.clone(),
                alert: alert.clone(),
            });
        }
        self.prompts.push_back(Prompt {
            serial,
            owner,
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
            (PromptAction::Save(id), Some(0)) => self.send(Command::PutBackAndSave(id)),
            (PromptAction::Save(id), Some(1)) => {
                self.effects.push_back(HostEffect::SaveAs(id));
                Ok(())
            }
            _ => Ok(()),
        };
        if let Some(next) = self.prompts.front() {
            self.effects.push_back(HostEffect::Alert {
                serial: next.serial,
                owner: next.owner.clone(),
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
    pub fn undo_label(&self) -> Option<&'static str> {
        (!self.undo_trash.is_empty()).then_some("Undo Move to Trash")
    }
    pub fn undo_move_to_trash(&mut self) -> Result<(), Failure> {
        if let Some(id) = self.undo_trash.last().cloned() {
            self.send(Command::PutBack(id))?;
        }
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
                        self.undo_trash.retain(|id| {
                            self.snapshot
                                .rows
                                .iter()
                                .any(|row| &row.id == id && row.trashed)
                        });
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
                            // The duplicate may be outside the current filter. Do
                            // not silently change the user's search/section.
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
                                self.effects.push_back(HostEffect::IntegrationGap("DESIGN-TODO(opus): reveal/select a duplicate outside the current filter without guessing a filter change."));
                            }
                        }
                        for row in self.snapshot.rows.iter().filter(|r| r.open) {
                            self.effects.push_back(HostEffect::Subtitle {
                                id: row.id.clone(),
                                text: row.trashed.then_some("In Trash"),
                            });
                        }
                    }
                    _ => {
                        self.failed = true;
                        self.effects.push_back(HostEffect::Announce {
                            text: "Your projects can’t be shown right now.".into(),
                            high: true,
                        });
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
                        Command::Trash(id) => self.undo_trash.push(id.clone()),
                        Command::PutBack(id) => self.undo_trash.retain(|x| x != id),
                        Command::Create(_) => {
                            self.form = None;
                            self.effects.push_back(HostEffect::CloseNewProject);
                        }
                        _ => {}
                    }
                    match outcome {
                        Outcome::Duplicated(id) => self.pending_duplicate = Some(id),
                        Outcome::Opened { reply, new_window } => self
                            .effects
                            .push_back(HostEffect::OpenEditor { reply, new_window }),
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
                        | Command::Open { id, .. }
                        | Command::Trash(id)
                        | Command::PutBack(id)
                        | Command::Archive(id)
                        | Command::PutBackAndSave(id) => {
                            self.row(id).map_or(String::new(), |r| r.name.clone())
                        }
                        Command::Load(_) => String::new(),
                    };
                    let code = if let Failure::Service(code) = failure {
                        code
                    } else {
                        ErrorCode::Unavailable
                    };
                    let alert = Alert::error(code, &name);
                    self.effects.push_back(HostEffect::Announce {
                        text: alert.message.clone(),
                        high: true,
                    });
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

/// Mapping is deliberately finite; ⇧⌘O remains Open, never Project Library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Return,
    Escape,
    Find,
    Trash,
    Undo,
    NewProject,
    Other,
}
pub const KEYBOARD: &[(&str, Key)] = &[
    ("Left", Key::Left),
    ("Right", Key::Right),
    ("Up", Key::Up),
    ("Down", Key::Down),
    ("Return", Key::Return),
    ("Escape", Key::Escape),
    ("⌘F", Key::Find),
    ("⌘⌫", Key::Trash),
    ("⌘Z", Key::Undo),
    ("⌥⌘N", Key::NewProject),
];
