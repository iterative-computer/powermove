//! The running app's session slot. The app owns the kernel actor and rebinds the
//! current project from the UI; `Automation` attaches to this instead of opening
//! a second kernel. All waits here belong on MCP workers, never the main thread.
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
    current: Mutex<Option<SessionHandle>>,
    operations: Arc<AtomicBool>,
    /// The project whose open waits on a person's recovery choice.
    deciding: Mutex<Option<String>>,
    pub closing: AtomicBool,
    changed: Mutex<Vec<async_channel::Sender<()>>>,
}
/// An owned, nonblocking fence for app session transitions. Never held by a UI gesture.
pub struct Operation(Arc<AtomicBool>);
impl Drop for Operation {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Host {
    pub fn new(kernel: KernelHandle, profile: PathBuf) -> (Arc<Self>, async_channel::Receiver<()>) {
        let (changed, rx) = async_channel::bounded(1);
        (
            Arc::new(Self {
                kernel,
                profile,
                current: Mutex::new(None),
                operations: Arc::new(AtomicBool::new(false)),
                deciding: Mutex::new(None),
                closing: AtomicBool::new(false),
                changed: Mutex::new(vec![changed]),
            }),
            rx,
        )
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
    pub fn current(&self) -> Option<SessionHandle> {
        self.current.lock().unwrap().clone()
    }
    pub fn bind(&self, session: Option<SessionHandle>) {
        {
            let mut current = self.current.lock().unwrap();
            if *current == session {
                return;
            }
            *current = session;
        }
        self.changed
            .lock()
            .unwrap()
            .retain(|tx| !matches!(tx.try_send(()), Err(async_channel::TrySendError::Closed(_))));
    }
    pub fn subscribe(&self) -> async_channel::Receiver<()> {
        let (tx, rx) = async_channel::bounded(1);
        let _ = tx.try_send(());
        self.changed.lock().unwrap().push(tx);
        rx
    }
    pub fn shell(&self, call: ShellCall) -> Result<ShellReply> {
        self.kernel
            .shell(call)
            .wait()
            .ok_or_else(stopped)?
            .map_err(Error::kernel)
    }
    pub fn open(&self, id: String, source: SessionSource) -> Result<Value> {
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
        if let Some(old) = self.current().filter(|s| {
            s != &session
                && self
                    .kernel
                    .published(&s.project)
                    .is_some_and(|p| p.session.as_ref().map(|m| &m.handle) == Some(s))
        }) {
            match self.shell(ShellCall::Close(old)) {
                Ok(ShellReply::Closed {
                    recovery_copy: None,
                }) => {}
                other => {
                    // Do not strand the newly opened project if replacing the old one fails.
                    let _ = self.shell(ShellCall::DeferClose(session));
                    return Err(Error::kernel(other));
                }
            }
        }
        self.bind(Some(session.clone()));
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
