//! The running app's windows and the project each one shows. The app owns the
//! kernel actor; each editor window owns its project and reports it here, and
//! the key window is where MCP acts. `Automation` attaches to this instead of
//! opening a second kernel. All waits here belong on MCP workers, never main.
use crate::{Error, Result};
use pm_kernel::{
    KernelHandle, Published,
    session::{SessionHandle, SessionSource, ShellCall, ShellReply},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Host {
    pub kernel: KernelHandle,
    pub profile: PathBuf,
    windows: Mutex<Windows>,
    operations: Arc<AtomicBool>,
    /// The project whose open waits on a person's recovery choice.
    deciding: Mutex<Option<String>>,
    pending_opens: Mutex<crate::recovery::PendingOpens>,
    pub closing: AtomicBool,
}
/// Slot 0 is a windowless host's (the headless MCP server): its only project.
const WINDOWLESS: u64 = 0;
#[derive(Default)]
struct Windows {
    /// Each editor window's project, and how to tell that window MCP changed it.
    slots: std::collections::BTreeMap<u64, Slot>,
    /// The key editor window: where MCP opens and which project it means by default.
    active: Option<u64>,
}
#[derive(Default)]
struct Slot {
    /// What the window shows (it reports it).
    shown: Option<SessionHandle>,
    /// What MCP placed there and the window hasn't taken yet.
    placed: Option<SessionHandle>,
    changed: Option<async_channel::Sender<()>>,
}
impl Slot {
    /// What the window shows, or is about to.
    fn session(&self) -> Option<&SessionHandle> {
        self.placed.as_ref().or(self.shown.as_ref())
    }
}
impl Windows {
    fn target(&self) -> u64 {
        self.active
            .filter(|w| self.slots.contains_key(w))
            .or_else(|| self.slots.keys().copied().find(|w| *w != WINDOWLESS))
            .unwrap_or(WINDOWLESS)
    }
    /// Where MCP opens a project: the key window if it's empty, else an empty
    /// window, else the key window (whose project it replaces, as v1's one
    /// window did).
    fn open_target(&self) -> u64 {
        let target = self.target();
        let empty = |w: &u64| self.slots.get(w).is_some_and(|s| s.session().is_none());
        if empty(&target) {
            return target;
        }
        self.slots
            .keys()
            .copied()
            .find(|w| *w != WINDOWLESS && empty(w))
            .unwrap_or(target)
    }
    fn showing(&self, project: &str) -> Option<(u64, SessionHandle)> {
        self.slots.iter().find_map(|(w, slot)| {
            [&slot.placed, &slot.shown]
                .into_iter()
                .flatten()
                .find(|s| s.project == project)
                .map(|s| (*w, s.clone()))
        })
    }
}
/// An owned, nonblocking fence for app session transitions. Never held by a UI gesture.
pub struct Operation(Arc<AtomicBool>);
impl Drop for Operation {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Host {
    pub fn new(kernel: KernelHandle, profile: PathBuf) -> Arc<Self> {
        let mut windows = Windows::default();
        windows.slots.insert(WINDOWLESS, Slot::default());
        Arc::new(Self {
            kernel,
            profile,
            windows: Mutex::new(windows),
            operations: Arc::new(AtomicBool::new(false)),
            deciding: Mutex::new(None),
            pending_opens: Mutex::default(),
            closing: AtomicBool::new(false),
        })
    }
    /// An editor window opened. The receiver wakes when MCP changes its project.
    pub fn attach_window(&self, window: u64) -> async_channel::Receiver<()> {
        assert_ne!(window, WINDOWLESS, "window ids start at 1");
        let (tx, rx) = async_channel::bounded(1);
        let mut windows = self.windows.lock().unwrap();
        windows.slots.insert(
            window,
            Slot {
                changed: Some(tx),
                ..Slot::default()
            },
        );
        windows.active.get_or_insert(window);
        rx
    }
    /// An editor window closed; its project is no longer anyone's.
    pub fn detach_window(&self, window: u64) {
        let mut windows = self.windows.lock().unwrap();
        windows.slots.remove(&window);
        if windows.active == Some(window) {
            windows.active = None;
        }
    }
    /// The key editor window changed: MCP acts there from now on.
    pub fn activate_window(&self, window: u64) {
        let mut windows = self.windows.lock().unwrap();
        if windows.slots.contains_key(&window) {
            windows.active = Some(window);
        }
    }
    /// What a window shows, as it reports it. Never notifies anyone: a window's
    /// project is its own, and other windows don't follow it.
    pub fn bind_window(&self, window: u64, session: Option<SessionHandle>) {
        if let Some(slot) = self.windows.lock().unwrap().slots.get_mut(&window) {
            slot.shown = session;
        }
    }
    /// What `window` shows, or MCP placed there for it to show.
    pub fn window_session(&self, window: u64) -> Option<SessionHandle> {
        self.windows
            .lock()
            .unwrap()
            .slots
            .get(&window)
            .and_then(|s| s.session().cloned())
    }
    /// The window takes the project MCP placed for it, before showing it: from
    /// here on it is the window's, and a later MCP open can't displace it.
    pub fn claim_placed(&self, window: u64) -> Option<SessionHandle> {
        let mut windows = self.windows.lock().unwrap();
        let slot = windows.slots.get_mut(&window)?;
        let placed = slot.placed.take()?;
        slot.shown = Some(placed.clone());
        Some(placed)
    }
    /// The window showing `project`, if one does.
    pub fn window_of(&self, project: &str) -> Option<u64> {
        let windows = self.windows.lock().unwrap();
        windows
            .showing(project)
            .map(|(w, _)| w)
            .filter(|w| *w != WINDOWLESS)
    }
    /// The session of `project` in whichever window shows it.
    pub fn session_of(&self, project: &str) -> Option<SessionHandle> {
        self.windows
            .lock()
            .unwrap()
            .showing(project)
            .map(|(_, s)| s)
    }
    pub fn try_operation(&self) -> Option<Operation> {
        self.operations
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Operation(self.operations.clone()))
    }
    /// Whether a session transition holds the fence now.
    pub fn operation_in_progress(&self) -> bool {
        self.operations.load(Ordering::Acquire)
    }
    /// While `project`'s open waits on a recovery choice in the window, tools
    /// say so instead of reporting a bare operation in progress.
    pub fn awaiting_recovery(&self, project: Option<String>) {
        *self.deciding.lock().unwrap() = project;
    }
    /// The reply to a tool the open's fence turned away.
    pub fn fenced(&self) -> Error {
        match self.deciding.lock().unwrap().as_deref() {
            Some(project) => Error::conflict(format!(
                "“{project}” has unsaved work from a session that ended unexpectedly. \
                 Powermove is waiting for someone to choose what to do with it in its \
                 window, and nothing has changed yet. list_projects and the Store tools \
                 still work. Ask them to choose, then try again."
            )),
            None => Error::conflict("A project operation is in progress. Inspect and retry."),
        }
    }
    /// The project MCP means by default: the key window's.
    pub fn current(&self) -> Option<SessionHandle> {
        let windows = self.windows.lock().unwrap();
        windows
            .slots
            .get(&windows.target())
            .and_then(|s| s.session().cloned())
    }
    /// MCP places `session` in its window (see `open_target`), which then shows
    /// it and becomes MCP's. Returns the session it displaces and nobody shows:
    /// a windowless host's previous project, or a window's earlier placement it
    /// never took (two opens before the window caught up).
    fn place(&self, session: SessionHandle) -> (u64, Option<SessionHandle>) {
        let mut windows = self.windows.lock().unwrap();
        let target = windows.open_target();
        if target != WINDOWLESS {
            windows.active = Some(target);
        }
        let slot = windows.slots.get_mut(&target).expect("target slot");
        let displaced = if target == WINDOWLESS {
            slot.shown.replace(session.clone())
        } else {
            slot.placed.replace(session.clone())
        };
        if let Some(tx) = &slot.changed {
            let _ = tx.try_send(());
        }
        let displaced = displaced.filter(|old| {
            old != &session && windows.showing(&old.project).is_none_or(|(_, s)| &s != old)
        });
        (target, displaced)
    }
    pub fn shell(&self, call: ShellCall) -> Result<ShellReply> {
        self.kernel
            .shell(call)
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)
    }
    /// MCP open_project/create_project. A project a window already shows stays
    /// there and that window becomes MCP's. Otherwise it opens in an empty
    /// window, else in the key window, which closes what it showed itself: MCP
    /// never closes a window's project underneath it.
    pub fn open(&self, id: String, source: SessionSource) -> Result<Value> {
        if let Some(window) = self.window_of(&id) {
            self.activate_window(window);
            let p = self.kernel.published(&id).ok_or_else(stopped)?;
            return Ok(identity(&p));
        }
        let reply = self.shell(ShellCall::PrepareOpen {
            id: id.clone(),
            source,
            electron_profile: None,
        })?;
        self.finish_open(&id, reply)
    }
    pub fn pending_recoveries(&self) -> Value {
        self.pending_opens.lock().unwrap().list()
    }
    pub fn decide_recovery(&self, id: &str, choice: &Value) -> Result<Value> {
        let (ticket, decision) = self.pending_opens.lock().unwrap().take(id, choice)?;
        let reply = self.shell(ShellCall::DecideRecovery { ticket, decision })?;
        self.finish_open(id, reply)
    }
    fn finish_open(&self, id: &str, reply: ShellReply) -> Result<Value> {
        if let ShellReply::RecoveryRequired { ticket, recovery } = reply {
            return Ok(self.pending_opens.lock().unwrap().required(id, ticket, recovery));
        }
        let ShellReply::Opened {
            session,
            decision: pm_kernel::locks::OpenDecision::Writable,
            ..
        } = reply
        else {
            return Err(Error::kernel(reply));
        };
        let (target, old) = self.place(session.clone());
        // A window closes the project it showed once it shows the new one; MCP
        // closes only what no window shows (see `place`).
        if let Some(old) = old.filter(|s| {
            self.kernel
                .published(&s.project)
                .is_some_and(|p| p.session.as_ref().map(|m| &m.handle) == Some(s))
        }) {
            match self.shell(ShellCall::Close(old.clone())) {
                Ok(ShellReply::Closed {
                    recovery_copy: None,
                }) => {}
                other if target == WINDOWLESS => {
                    // Do not strand the newly opened project if replacing the old one fails.
                    self.bind_window(WINDOWLESS, Some(old));
                    let _ = self.shell(ShellCall::DeferClose(session));
                    return Err(Error::kernel(other));
                }
                // The displaced placement keeps its recovery journal; the new
                // project still opens in the window.
                _ => {}
            }
        }
        let p = self
            .kernel
            .published(&session.project)
            .ok_or_else(stopped)?;
        Ok(identity(&p))
    }
    /// EOF from a headless owner closes library projects durably. Unsaved
    /// named files keep their journal, like a deferred close in the app.
    /// A proxy disconnect must never call this on a windowed host.
    pub fn close_headless(&self) -> Result<()> {
        let current = {
            let windows = self.windows.lock().unwrap();
            if windows.slots.keys().any(|w| *w != WINDOWLESS) {
                return Err(Error::invalid("Cannot close a headless host with editor windows"));
            }
            windows.slots.get(&WINDOWLESS).and_then(|s| s.session()).cloned()
        };
        if let Some(session) = current {
            let p = self.kernel.published(&session.project).ok_or_else(stopped)?;
            let call = if p.path.is_some() && p.dirty {
                ShellCall::DeferClose(session)
            } else {
                ShellCall::Close(session)
            };
            match self.shell(call)? {
                ShellReply::Closed { recovery_copy: None } => self.bind_window(WINDOWLESS, None),
                other => return Err(Error::kernel(other)),
            }
        }
        self.shell(ShellCall::ReleaseLibrary)?;
        Ok(())
    }
    /// Save a project whose document has a named working file, in place.
    pub(crate) fn save_document(&self, session: SessionHandle, document: u64) -> Result<()> {
        let save = self
            .kernel
            .prepare_document_save(session, document)
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)?;
        let reply = self
            .kernel
            .finish_save(save.run())
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)?;
        match reply {
            ShellReply::Saved {
                recovery_copy: None,
                ..
            } => Ok(()),
            other => Err(Error::kernel(other)),
        }
    }
    pub fn save(&self, session: SessionHandle, path: Option<PathBuf>) -> Result<Value> {
        let p = self
            .kernel
            .published(&session.project)
            .ok_or_else(stopped)?;
        let file = path.is_some() || p.path.is_some();
        let reply = if file {
            let save = self
                .kernel
                .prepare_save(session, path)
                .wait()
                .ok_or_else(stopped)?
                .map_err(Error::kernel)?;
            self.kernel
                .finish_save(save.run())
                .wait()
                .ok_or_else(stopped)?
                .map_err(Error::kernel)?
        } else {
            self.shell(ShellCall::Save(session))?
        };
        match reply {
            ShellReply::Saved {
                session,
                recovery_copy: None,
                ..
            } => {
                let p = self
                    .kernel
                    .published(&session.project)
                    .ok_or_else(stopped)?;
                Ok(json!({"projectId":session.project,"path":p.path}))
            }
            other => Err(Error::kernel(other)),
        }
    }
}
/// The kernel dropped the call's answer: an internal error interrupted it
/// (contained, in the Error Log; its project stops), or the app is quitting.
pub(crate) fn stopped() -> Error {
    Error {
        code: "kernel_error".into(),
        message: "The app kernel didn’t answer: an internal error interrupted this call (see the \
                  Error Log), or the app is quitting."
            .into(),
        backing: Some("pm-kernel".into()),
    }
}
fn identity(p: &Published) -> Value {
    let name = p
        .snapshot
        .root()
        .and_then(|root| root.get("name"))
        .and_then(|node| match node.kind() {
            pm_model::NodeKind::String(text) => text.as_str(),
            _ => None,
        });
    json!({"projectId":p.project,"name":name})
}
