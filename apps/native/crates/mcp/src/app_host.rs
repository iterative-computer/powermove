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
struct Slot {
    session: Option<SessionHandle>,
    changed: Option<async_channel::Sender<()>>,
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
        let empty = |w: &u64| self.slots.get(w).is_some_and(|s| s.session.is_none());
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
            slot.session
                .clone()
                .filter(|s| s.project == project)
                .map(|s| (*w, s))
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
        windows.slots.insert(
            WINDOWLESS,
            Slot {
                session: None,
                changed: None,
            },
        );
        Arc::new(Self {
            kernel,
            profile,
            windows: Mutex::new(windows),
            operations: Arc::new(AtomicBool::new(false)),
            deciding: Mutex::new(None),
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
                session: None,
                changed: Some(tx),
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
            slot.session = session;
        }
    }
    /// The project MCP placed in (or a window reported for) `window`.
    pub fn window_session(&self, window: u64) -> Option<SessionHandle> {
        self.windows
            .lock()
            .unwrap()
            .slots
            .get(&window)
            .and_then(|s| s.session.clone())
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
            .and_then(|s| s.session.clone())
    }
    /// MCP places `session` in its window (see `open_target`), which then shows
    /// it and becomes MCP's. Returns the session that window showed before.
    fn place(&self, session: SessionHandle) -> (u64, Option<SessionHandle>) {
        let mut windows = self.windows.lock().unwrap();
        let target = windows.open_target();
        if target != WINDOWLESS {
            windows.active = Some(target);
        }
        let slot = windows.slots.get_mut(&target).expect("target slot");
        let old = slot.session.replace(session);
        if let Some(tx) = &slot.changed {
            let _ = tx.try_send(());
        }
        (target, old)
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
            id,
            source,
            electron_profile: None,
        })?;
        // Windowless clients never choose a recovery policy on the user's behalf.
        let ShellReply::Opened {
            session,
            decision: pm_kernel::locks::OpenDecision::Writable,
            ..
        } = reply
        else {
            return Err(Error::kernel(reply));
        };
        let (target, old) = self.place(session.clone());
        // Only a windowless host closes the project it replaces; a window
        // closes its own once it shows the new one.
        if target == WINDOWLESS
            && let Some(old) = old.filter(|s| {
                s != &session
                    && self
                        .kernel
                        .published(&s.project)
                        .is_some_and(|p| p.session.as_ref().map(|m| &m.handle) == Some(s))
            })
        {
            match self.shell(ShellCall::Close(old.clone())) {
                Ok(ShellReply::Closed {
                    recovery_copy: None,
                }) => {}
                other => {
                    // Do not strand the newly opened project if replacing the old one fails.
                    self.bind_window(WINDOWLESS, Some(old));
                    let _ = self.shell(ShellCall::DeferClose(session));
                    return Err(Error::kernel(other));
                }
            }
        }
        let p = self
            .kernel
            .published(&session.project)
            .ok_or_else(stopped)?;
        Ok(identity(&p))
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
pub(crate) fn stopped() -> Error {
    Error::unavailable("app kernel stopped")
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
