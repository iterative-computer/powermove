//! S002/S003/S004/S014/S016/S020/S055, design/surfaces/documents.md.
//!
//! N1/N3: presentation descriptions contain only Opus-specified copy/semantics.
//! N4: discard uses the kernel archive operation; no source is removed.
//! N7: requests carry stable window/session IDs and monotonically unique tickets.
//! N8: hosts enqueue work; only workers may execute or wait for kernel/file IO.
//! `lifecycle.rs` owns the close and quit sequence; alerts and notices come from here.
use pm_kernel::{
    Published,
    locks::{CopyReason, OpenDecision},
    session::{SessionHandle, ShellCall, ShellReply},
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    time::Duration,
};

#[cfg(target_os = "macos")]
#[path = "documents_appkit.rs"]
pub mod appkit;
#[cfg(test)]
#[path = "documents_tests.rs"]
mod tests;
#[path = "documents_windows.rs"]
pub mod windows;

pub type WindowId = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    Window(WindowId),
    Application,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket(pub u64);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub owner: Owner,
    pub session: Option<SessionHandle>,
}
#[derive(Default)]
pub struct Tickets {
    _private: (),
}
impl Tickets {
    pub fn next_ticket(&mut self) -> Ticket {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut previous = NEXT.load(std::sync::atomic::Ordering::Relaxed);
        loop {
            let next = previous.checked_add(1).expect("surface ticket exhaustion");
            match NEXT.compare_exchange_weak(
                previous,
                next,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            ) {
                Ok(_) => return Ticket(previous),
                Err(observed) => previous = observed,
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Save,
    Cancel,
    DontSave,
    ReviewChanges,
    DiscardChanges,
    SaveAs,
    SaveAnyway,
    ShowInFinder,
    Ok,
    ShowProjectLibrary,
    SaveAgain,
    Dismiss,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Return,
    Escape,
    CommandD,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub action: Action,
    pub label: &'static str,
    pub key: Option<Key>,
    pub destructive: bool,
}
impl Button {
    pub fn new(action: Action, label: &'static str, key: Option<Key>) -> Self {
        Self {
            action,
            label,
            key,
            destructive: false,
        }
    }
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub message: String,
    pub informative: String,
    pub buttons: Vec<Button>,
}
impl Alert {
    pub fn action_for_key(&self, key: Key) -> Option<Action> {
        self.buttons
            .iter()
            .find(|b| b.key == Some(key))
            .map(|b| b.action)
    }
    /// Aborted/closed alerts never choose a destructive or affirmative action.
    pub fn action_at(&self, index: Option<usize>) -> Option<Action> {
        index
            .and_then(|i| self.buttons.get(i))
            .map(|b| b.action)
            .or_else(|| {
                self.buttons
                    .iter()
                    .find(|b| b.action == Action::Cancel)
                    .map(|b| b.action)
            })
    }
}
fn cancel() -> Button {
    Button::new(Action::Cancel, "Cancel", Some(Key::Escape))
}
fn default(action: Action, label: &'static str) -> Button {
    Button::new(action, label, Some(Key::Return))
}
pub fn unsaved(name: &str) -> Alert {
    Alert {
        message: format!("Save changes to “{name}” before closing?"),
        informative: "Your project file hasn’t been updated. Local recovery is kept separately."
            .into(),
        buttons: vec![
            default(Action::Save, "Save"),
            cancel(),
            Button::new(Action::DontSave, "Don’t Save", Some(Key::CommandD)).destructive(),
        ],
    }
}
pub fn review_quit(count: usize) -> Alert {
    assert!(count >= 2);
    Alert { message: format!("You have {count} projects with unsaved changes. Do you want to review these changes before quitting?"), informative: "If you discard them, their project files aren’t updated. Local recovery is kept separately.".into(), buttons: vec![default(Action::ReviewChanges,"Review Changes…"), cancel(), Button::new(Action::DiscardChanges,"Discard Changes",None).destructive()] }
}
pub fn changed_on_disk(name: &str) -> Alert {
    Alert {
        message: format!("“{name}” was changed by another app since you opened or saved it."),
        informative: "If you save, the other version is kept as a recovery copy.".into(),
        buttons: vec![
            default(Action::SaveAnyway, "Save Anyway"),
            cancel(),
            Button::new(Action::SaveAs, "Save As…", None),
        ],
    }
}
/// Wire spelling is ServiceErrorCode's serde snake_case, never Display/diagnostics.
/// Unknown future codes intentionally use the reviewed generic error.
pub fn service_error(code: &str, name: &str) -> (String, &'static str) {
    match code {
        "offline" => (
            "Powermove can’t connect.".into(),
            "Check your internet connection and try again.",
        ),
        "not_found" => (
            format!("“{name}” can’t be found."),
            "It may have been moved or deleted.",
        ),
        "too_large" => (format!("“{name}” is too large to open."), ""),
        "invalid_project" => (
            format!("“{name}” can’t be opened because it isn’t a Powermove project."),
            "",
        ),
        "permission_denied" => (
            format!("You don’t have permission to open “{name}”."),
            "Check its sharing and permissions in Finder.",
        ),
        "disk_full" => (
            "There isn’t enough disk space.".into(),
            "Free some space and try again.",
        ),
        "journal" => (
            format!("“{name}” couldn’t be opened."),
            "Powermove couldn’t start the recovery journal that protects unsaved work. Check free space and permissions in your library, then try again.",
        ),
        "invalid_input" | "invalid_data" => (
            format!("“{name}” couldn’t be read."),
            "The file may be damaged.",
        ),
        _ => (
            "The operation couldn’t be completed.".into(),
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    }
}
pub fn failed_save(name: &str, code: &str) -> Alert {
    Alert {
        message: format!("“{name}” couldn’t be saved."),
        informative: service_error(code, name).1.into(),
        buttons: vec![default(Action::SaveAs, "Save As…"), cancel()],
    }
}
pub fn open_error(name: &str, code: &str, path_exists: bool) -> Alert {
    let (message, informative) = service_error(code, name);
    // README §1 first is the default. S004 explicitly makes OK default even
    // though Show in Finder is written first in prose: retain its explicit key.
    let mut buttons = vec![default(Action::Ok, "OK")];
    if path_exists {
        buttons.push(Button::new(Action::ShowInFinder, "Show in Finder", None));
    }
    Alert {
        message,
        informative: informative.into(),
        buttons,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnouncementPriority {
    Medium,
    High,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub level: Level,
    pub action: Option<(Action, &'static str)>,
    /// A recovery id is not necessarily a path. Resolve it on a worker first.
    pub recovery_id: Option<String>,
    pub priority: AnnouncementPriority,
}
impl Notice {
    pub fn viewer_timeout(&self) -> Option<Duration> {
        self.action.is_none().then_some(Duration::from_secs(4))
    }
    pub fn dismiss_label(&self) -> Option<&'static str> {
        self.action.map(|_| "Dismiss")
    }
}
pub fn reloaded(name: &str) -> Notice {
    Notice {
        text: format!("“{name}” was updated from another app."),
        level: Level::Info,
        action: None,
        recovery_id: None,
        priority: AnnouncementPriority::Medium,
    }
}
pub fn recovery_copy(name: &str, id: String) -> Notice {
    Notice {
        text: format!("A recovery copy of “{name}” was kept."),
        level: Level::Info,
        action: Some((Action::ShowInFinder, "Show in Finder")),
        recovery_id: Some(id),
        priority: AnnouncementPriority::Medium,
    }
}
pub fn uncertain_save(name: &str) -> Notice {
    Notice {
        text: format!("Couldn’t confirm that “{name}” was saved."),
        level: Level::Warning,
        action: Some((Action::SaveAgain, "Save Again")),
        recovery_id: None,
        priority: AnnouncementPriority::High,
    }
}
pub fn copy_reason(name: &str, reason: &CopyReason) -> String {
    let reason = match reason {
        CopyReason::Locked => "is open in Powermove on another Mac, so this is a copy.",
        CopyReason::UncertainLock => "may be open elsewhere, so this is a copy.",
        CopyReason::ReadOnly => "is read-only, so this is a copy.",
        CopyReason::CloudFolder => "is in a cloud folder that’s still syncing, so this is a copy.",
        CopyReason::OfflineFile => "isn’t downloaded, so this is a copy of the last local version.",
        CopyReason::ChangedOnDisk => "changed while it was opening, so this is a copy.",
        CopyReason::OlderElectron => "is open in an older Powermove, so this is a copy.",
        CopyReason::ForeignLibrary => "belongs to another Powermove library, so this is a copy.",
    };
    format!("“{name}” {reason}")
}
pub fn copy_notice(name: &str, decision: &OpenDecision) -> Option<Notice> {
    let OpenDecision::AsCopy(reason) = decision else {
        return None;
    };
    Some(Notice {
        text: copy_reason(name, reason),
        level: Level::Info,
        action: Some((Action::SaveAs, "Save As…")),
        recovery_id: None,
        priority: AnnouncementPriority::Medium,
    })
}
pub fn recovery_notices(name: &str, reply: &ShellReply) -> Vec<Notice> {
    match reply {
        ShellReply::Saved {
            recovery_copy: Some(id),
            ..
        }
        | ShellReply::Closed {
            recovery_copy: Some(id),
        } => vec![recovery_copy(name, id.clone())],
        _ => vec![],
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Saving,
    Importing,
    Durability,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finishing {
    pub title: &'static str,
    pub line: String,
    pub cancel: Button,
}
impl Finishing {
    pub fn new(name: &str, work: Work) -> Self {
        Self {
            title: "Finishing Up…",
            line: if work == Work::Importing {
                format!("Powermove is importing media into “{name}”.")
            } else {
                format!("Powermove is saving “{name}”.")
            },
            cancel: cancel(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Open,
    Save { name: String },
    Import { supported_types: Vec<String> },
    Folder,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub kind: PickerKind,
    pub title: &'static str,
    pub prompt: &'static str,
    pub files: bool,
    pub folders: bool,
    pub multiple: bool,
    pub create_directories: bool,
}
impl Picker {
    pub fn new(kind: PickerKind) -> Self {
        let (title, prompt, files, folders, multiple, create_directories) = match &kind {
            PickerKind::Open => ("Open", "Open", true, false, true, false),
            PickerKind::Save { .. } => ("Save As", "Save", true, false, false, true),
            PickerKind::Import { .. } => ("Import", "Import", true, true, true, false),
            PickerKind::Folder => ("Choose", "Choose", false, true, false, true),
        };
        Self {
            kind,
            title,
            prompt,
            files,
            folders,
            multiple,
            create_directories,
        }
    }
    pub fn save(name: &str, copy: bool) -> Self {
        Self::new(PickerKind::Save {
            name: if copy {
                format!("{name} copy")
            } else {
                name.into()
            },
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presentation {
    Alert(Alert),
    Picker(Picker),
    Finishing(Finishing),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub ticket: Ticket,
    pub target: Target,
    pub presentation: Presentation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Alert(Option<Action>),
    Paths(Vec<PathBuf>),
    Cancel,
    Unavailable(&'static str),
}
/// macOS implements system controls; tests inspect the same requests without UI.
pub trait DocumentHost {
    fn present(&mut self, request: Request);
    fn dismiss(&mut self, ticket: Ticket);
}
#[derive(Default)]
pub struct HeadlessHost {
    pub presented: Vec<Request>,
    pub dismissed: Vec<Ticket>,
}
impl DocumentHost for HeadlessHost {
    fn present(&mut self, request: Request) {
        self.presented.push(request);
    }
    fn dismiss(&mut self, ticket: Ticket) {
        self.dismissed.push(ticket);
    }
}
/// One system alert/sheet per owner, FIFO. Retains generations, not raw pointers.
#[derive(Default)]
pub struct PresentationQueue {
    active: BTreeMap<Owner, Request>,
    pending: BTreeMap<Owner, VecDeque<Request>>,
}
impl PresentationQueue {
    pub fn enqueue(&mut self, request: Request, host: &mut impl DocumentHost) {
        let owner = request.target.owner;
        if let std::collections::btree_map::Entry::Vacant(entry) = self.active.entry(owner) {
            entry.insert(request.clone());
            host.present(request);
        } else {
            self.pending.entry(owner).or_default().push_back(request);
        }
    }
    pub fn complete(&mut self, ticket: Ticket, host: &mut impl DocumentHost) -> Option<Request> {
        let owner = *self.active.iter().find(|(_, r)| r.ticket == ticket)?.0;
        let request = self.active.remove(&owner)?;
        if let Some(next) = self.pending.get_mut(&owner).and_then(VecDeque::pop_front) {
            self.active.insert(owner, next.clone());
            host.present(next);
        }
        Some(request)
    }
    /// Cancel an intent without cancelling the save/import it was waiting for.
    pub fn cancel(&mut self, ticket: Ticket, host: &mut impl DocumentHost) {
        if self.active.values().any(|r| r.ticket == ticket) {
            host.dismiss(ticket);
            self.complete(ticket, host);
        } else {
            for pending in self.pending.values_mut() {
                pending.retain(|r| r.ticket != ticket);
            }
        }
    }
    pub fn retire(&mut self, session: &SessionHandle, host: &mut impl DocumentHost) {
        for pending in self.pending.values_mut() {
            pending.retain(|r| r.target.session.as_ref() != Some(session));
        }
        let tickets: Vec<_> = self
            .active
            .values()
            .filter(|r| r.target.session.as_ref() == Some(session))
            .map(|r| r.ticket)
            .collect();
        for ticket in tickets {
            self.cancel(ticket, host);
        }
    }
}
#[derive(Clone, Debug)]
pub struct Document {
    pub session: SessionHandle,
    pub window: WindowId,
    pub name: String,
    pub dirty: bool,
    pub saving: bool,
    pub importing: bool,
    pub entry: bool,
    pub has_file: bool,
    pub copy: bool,
    /// Supplied by the file watcher; kernel has no watcher/reload API yet.
    pub disk_changed: bool,
}
impl Document {
    /// Import state belongs to the media host. Everything else comes from the
    /// kernel publication and its original OpenDecision (no second writer).
    pub fn from_published(
        p: &Published,
        window: WindowId,
        name: String,
        importing: bool,
        decision: &OpenDecision,
    ) -> Option<Self> {
        Some(Self {
            session: p.session.as_ref()?.handle.clone(),
            window,
            name,
            dirty: p.dirty,
            saving: p.saving,
            importing,
            entry: p.entry.is_some(),
            has_file: p.path.is_some(),
            copy: matches!(decision, OpenDecision::AsCopy(_)),
            disk_changed: false,
        })
    }
    pub fn subtitle(&self) -> &'static str {
        if self.saving {
            "Saving…"
        } else if self.copy && self.dirty {
            "Copy · Edited"
        } else if self.copy {
            "Copy"
        } else if self.dirty {
            "Edited"
        } else {
            ""
        }
    }
    pub fn work(&self) -> Option<Work> {
        if self.saving {
            Some(Work::Saving)
        } else if self.importing {
            Some(Work::Importing)
        } else if self.entry {
            Some(Work::Durability)
        } else {
            None
        }
    }
    pub fn target(&self) -> Target {
        Target {
            owner: Owner::Window(self.window),
            session: Some(self.session.clone()),
        }
    }
}
/// The production dispatch seam deliberately returns Pending; callers await it
/// on their executor and classify errors without copying diagnostics into UI.
pub fn submit(
    kernel: &pm_kernel::KernelHandle,
    call: ShellCall,
) -> pm_kernel::Pending<Result<ShellReply, pm_kernel::KernelFailure>> {
    kernel.shell(call)
}

/// Shared by Command-S and close-save, so copy-only sessions can never write
/// their original destination. A watcher must set disk_changed before preflight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavePreflight {
    Join(Work),
    Picker(Picker),
    ReviewChanged(Alert),
    Save,
}
pub fn save_preflight(document: &Document, force_save_as: bool) -> SavePreflight {
    if let Some(work) = document.work() {
        return SavePreflight::Join(work);
    }
    if force_save_as || document.copy || !document.has_file {
        return SavePreflight::Picker(Picker::save(&document.name, document.copy));
    }
    if document.disk_changed {
        return SavePreflight::ReviewChanged(changed_on_disk(&document.name));
    }
    SavePreflight::Save
}
/// Worker supplies path existence; there is no UI-thread filesystem access.
/// Preserve FileOpenFailure.path even if that path no longer exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFailure {
    pub path: PathBuf,
    pub alert: Alert,
}
impl FileFailure {
    pub fn from_service(
        path: PathBuf,
        display_name: &str,
        code_wire: &str,
        path_exists: bool,
    ) -> Self {
        Self {
            path,
            alert: open_error(display_name, code_wire, path_exists),
        }
    }
    pub fn finder_path(&self) -> Option<&std::path::Path> {
        self.alert
            .buttons
            .iter()
            .any(|b| b.action == Action::ShowInFinder)
            .then_some(self.path.as_path())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLocation {
    Viewer,
    Library,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeActivation {
    pub session: SessionHandle,
    pub action: Action,
    pub recovery_id: Option<String>,
}
/// Lifetime for one visible notice. The host supplies the single viewer slot;
/// this type does not invent a stacking/reordering policy for simultaneous facts.
pub struct NoticeSurface {
    pub session: SessionHandle,
    pub notice: Notice,
    pub location: NoticeLocation,
    pub visible: bool,
    started: Duration,
    announced: bool,
}
impl NoticeSurface {
    pub fn new(
        session: SessionHandle,
        notice: Notice,
        location: NoticeLocation,
        now: Duration,
    ) -> Self {
        Self {
            session,
            notice,
            location,
            visible: true,
            started: now,
            announced: false,
        }
    }
    pub fn take_announcement(&mut self) -> Option<(&str, AnnouncementPriority)> {
        if !self.visible || std::mem::replace(&mut self.announced, true) {
            return None;
        }
        Some((&self.notice.text, self.notice.priority))
    }
    /// Returns true only when the visible viewer banner expires this tick.
    pub fn tick(&mut self, now: Duration) -> bool {
        if self.visible
            && self.location == NoticeLocation::Viewer
            && self
                .notice
                .viewer_timeout()
                .is_some_and(|timeout| now.saturating_sub(self.started) >= timeout)
        {
            self.visible = false;
            true
        } else {
            false
        }
    }
    pub fn activate(
        &mut self,
        session: &SessionHandle,
        action: Action,
    ) -> Option<NoticeActivation> {
        if !self.visible || session != &self.session {
            return None;
        }
        let allowed = self.notice.action.is_some_and(|(declared, _)| {
            action == declared
                || (action == Action::Dismiss && self.location == NoticeLocation::Viewer)
        });
        if !allowed {
            return None;
        }
        self.visible = false;
        Some(NoticeActivation {
            session: session.clone(),
            action,
            recovery_id: self.notice.recovery_id.clone(),
        })
    }
    pub fn document_closed(&mut self, session: &SessionHandle) -> bool {
        if session != &self.session {
            return false;
        }
        std::mem::replace(&mut self.visible, false)
    }
}
