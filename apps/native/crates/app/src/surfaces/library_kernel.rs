//! Worker adapter. The kernel remains the only writer; LibraryStore is read-only.
use super::{controller::*, form::NewProject, model::*};
use pm_kernel::{
    KernelFailure, KernelHandle,
    library::{LibraryError, LibraryStore},
    session::{SessionSource, ShellCall, ShellReply},
};
use pm_model::json::Value;
use pm_services::ServiceErrorCode;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

pub struct KernelLibraryPort {
    tx: mpsc::SyncSender<Request>,
    rx: mpsc::Receiver<Completion>,
}
impl KernelLibraryPort {
    /// wake posts onto the UI executor; it must never call JS or wait for UI.
    /// No UI-thread file access, joins, or blocking sends (including Drop).
    pub fn start(
        kernel: KernelHandle,
        profile: PathBuf,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        let (tx, requests) = mpsc::sync_channel::<Request>(16);
        let (responses, rx) = mpsc::channel();
        thread::Builder::new()
            .name("pm-library".into())
            .spawn(move || {
                let mut service = KernelLibrary { kernel, profile };
                while let Ok(request) = requests.recv() {
                    let completion = Completion {
                        serial: request.serial,
                        result: service.execute(request.command),
                    };
                    if responses.send(completion).is_err() {
                        break;
                    }
                    wake();
                }
            })?;
        Ok(Self { tx, rx })
    }
}
impl LibraryPort for KernelLibraryPort {
    fn enqueue(&mut self, request: Request) -> Result<(), Failure> {
        self.tx.try_send(request).map_err(|e| match e {
            mpsc::TrySendError::Full(_) => Failure::Busy,
            mpsc::TrySendError::Disconnected(_) => Failure::Service(ServiceErrorCode::Unavailable),
        })
    }
    fn poll(&mut self) -> Option<Completion> {
        self.rx.try_recv().ok()
    }
}

/// Worker-only implementation, also usable by headless agents/tests. No
/// DocumentStore, LibraryWriter, direct disk mutation or second writer lease.
pub struct KernelLibrary {
    pub kernel: KernelHandle,
    pub profile: PathBuf,
}
impl KernelLibrary {
    fn save(&self, session: pm_kernel::session::SessionHandle) -> Result<(), Failure> {
        let file_backed = self
            .kernel
            .published(&session.project)
            .and_then(|p| {
                p.session.as_ref().map(|s| {
                    matches!(
                        s.destination,
                        pm_kernel::session::SessionDestination::File(_)
                    )
                })
            })
            .ok_or(Failure::Stale)?;
        if !file_backed {
            // The kernel's file handoff deliberately excludes library saves.
            return match self.shell(ShellCall::Save(session))? {
                ShellReply::Saved { .. } => Ok(()),
                _ => Err(Failure::Service(ServiceErrorCode::Unavailable)),
            };
        }
        let plan = self
            .kernel
            .prepare_save(session, None)
            .wait()
            .ok_or(Failure::Service(ServiceErrorCode::Unavailable))?
            .map_err(kernel_failure)?;
        // File packing and IO execute on this worker, outside the actor.
        let completion = plan.run();
        match self
            .kernel
            .finish_save(completion)
            .wait()
            .ok_or(Failure::Service(ServiceErrorCode::Unavailable))?
            .map_err(kernel_failure)?
        {
            ShellReply::Saved { .. } => Ok(()),
            _ => Err(Failure::Service(ServiceErrorCode::Unavailable)),
        }
    }
    fn shell(&self, call: ShellCall) -> Result<ShellReply, Failure> {
        self.kernel
            .shell(call)
            .wait()
            .ok_or(Failure::Service(ServiceErrorCode::Unavailable))?
            .map_err(kernel_failure)
    }
    pub fn execute(&mut self, command: Command) -> Result<Outcome, Failure> {
        match command {
            Command::Load(query) => {
                let snapshot = self.snapshot()?;
                let rows = query.apply(&snapshot);
                Ok(Outcome::Loaded { snapshot, rows })
            }
            Command::Open(id) => {
                validate_id(&id)?;
                if self
                    .kernel
                    .published(&id)
                    .is_some_and(|p| p.session.is_some())
                {
                    return Ok(Outcome::Focus(id));
                }
                let reply = self
                    .kernel
                    .shell(ShellCall::PrepareOpen {
                        id: id.clone(),
                        source: SessionSource::Library {
                            profile: self.profile.clone(),
                            project: id.clone(),
                        },
                        electron_profile: None,
                    })
                    .wait()
                    .ok_or(Failure::Service(ServiceErrorCode::Unavailable))?;
                // Another window opened it meanwhile: bring that window forward.
                let reply = match reply {
                    Err(KernelFailure::AlreadyOpen(_)) => return Ok(Outcome::Focus(id)),
                    other => other.map_err(kernel_failure)?,
                };
                match reply {
                    ShellReply::Opened { .. } | ShellReply::RecoveryRequired { .. } => {
                        Ok(Outcome::Opened(reply))
                    }
                    _ => Err(Failure::Service(ServiceErrorCode::Unavailable)),
                }
            }
            Command::Create(values) => {
                let id = pm_kernel::new_id("P");
                let project = project_value(&id, &values)?;
                let reply = self.shell(ShellCall::Open {
                    id,
                    source: SessionSource::New(project),
                    electron_profile: None,
                })?;
                if !matches!(reply, ShellReply::Opened { .. }) {
                    return Err(Failure::Service(ServiceErrorCode::Unavailable));
                }
                Ok(Outcome::Opened(reply))
            }
            Command::Duplicate { id, name } => {
                validate_id(&id)?;
                if name.trim().is_empty() {
                    return Err(Failure::Service(ServiceErrorCode::InvalidInput));
                }
                let new_id = pm_kernel::new_id("P");
                changed(self.shell(ShellCall::Duplicate {
                    project: id,
                    id: new_id.clone(),
                    name,
                })?)?;
                Ok(Outcome::Duplicated(new_id))
            }
            Command::Rename { id, name } => {
                validate_id(&id)?;
                if name.trim().is_empty() {
                    return Err(Failure::Service(ServiceErrorCode::InvalidInput));
                }
                let Some(published) = self.kernel.published(&id).filter(|p| p.session.is_some())
                else {
                    return Err(Failure::Unsupported(
                        "Kernel needs an unopened-slot rename operation; never open/save/close a hidden editing session to rename.",
                    ));
                };
                let session = published.session.as_ref().expect("session").handle.clone();
                let call = pm_kernel::tx::Call::Transact {
                    label: Some("Rename Project".into()),
                    body: pm_kernel::WriteBody::Commands(vec![
                        pm_kernel::commands::comps::CompCommand::Set {
                            patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                name: Some(name),
                                ..Default::default()
                            }),
                        }
                        .into(),
                    ]),
                    expected_revision: Some(published.revision()),
                };
                let ack = self
                    .kernel
                    .call(
                        &id,
                        pm_kernel::tx::Request::new(pm_kernel::tx::Caller::native(), call),
                    )
                    .wait()
                    .ok_or(Failure::Service(ServiceErrorCode::Unavailable))?
                    .map_err(kernel_failure)?;
                if matches!(ack.reply, pm_kernel::tx::Reply::Throw(_)) {
                    return Err(Failure::Stale);
                }
                self.save(session)?;
                Ok(Outcome::Changed)
            }
            Command::Trash(id) => {
                validate_id(&id)?;
                if self
                    .kernel
                    .published(&id)
                    .is_some_and(|p| p.session.is_some())
                {
                    return Err(Failure::Unsupported(
                        "Kernel Trash currently refuses open projects; add safe open-session Trash without closing or losing edits.",
                    ));
                }
                changed(self.shell(ShellCall::Trash(id.clone()))?)?;
                // Its Home poster goes with it; Put Back renders a new one at
                // the project's next save or close.
                crate::poster::forget(&self.profile.join("caches/thumbnails"), &id);
                Ok(Outcome::Changed)
            }
            Command::PutBack(id) => {
                validate_id(&id)?;
                changed(self.shell(ShellCall::Restore(id))?)?;
                Ok(Outcome::Changed)
            }
            Command::Archive(id) => {
                validate_id(&id)?;
                Err(Failure::Unsupported(
                    "Kernel needs ArchiveReason-backed Delete Immediately, preserving all project/history/media bytes and excluding archived IDs from Library; no filesystem deletion fallback.",
                ))
            }
        }
    }
    pub fn snapshot(&self) -> Result<Snapshot, Failure> {
        let ShellReply::Recent(recent) = self.shell(ShellCall::Recent)? else {
            return Err(Failure::Service(ServiceErrorCode::Unavailable));
        };
        let store = LibraryStore::open(&self.profile);
        let trash = store.trash().map_err(library_failure)?;
        let mut rows = Vec::with_capacity(recent.len() + trash.len());
        let mut recents = BTreeSet::new();
        for raw in recent {
            let row = self.row(&store, raw, false)?;
            recents.insert(row.id.clone());
            rows.push(row);
        }
        for entry in trash {
            let row = self.row(&store, entry.raw, true)?;
            // Trashed by any route (Home, an agent): its poster goes too.
            crate::poster::forget(&self.profile.join("caches/thumbnails"), &row.id);
            rows.push(row);
        }
        Ok(Snapshot { rows, recents })
    }
    fn row(&self, store: &LibraryStore, raw: Value, trashed: bool) -> Result<Row, Failure> {
        let id = raw
            .get("id")
            .and_then(Value::as_str)
            .ok_or(Failure::Service(ServiceErrorCode::InvalidData))?
            .to_owned();
        validate_id(&id)?;
        let name = raw
            .get("name")
            .and_then(Value::as_str)
            .ok_or(Failure::Service(ServiceErrorCode::InvalidData))?
            .to_owned();
        let mut row = Row {
            modified_ms: raw
                .get("at")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= i64::MIN as f64 && *n < i64::MAX as f64)
                .map(|n| n as i64),
            deleted_ms: raw
                .get("deletedAt")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= i64::MIN as f64 && *n < i64::MAX as f64)
                .map(|n| n as i64),
            summary: Some(Summary {
                thumbnail: raw.get("thumb").and_then(Value::as_str).map(str::to_owned),
                duration_seconds: number(&raw, "dur"),
                width: dimension(&raw, "w"),
                height: dimension(&raw, "h"),
                fps: number(&raw, "fps"),
            }),
            id: id.clone(),
            name,
            trashed,
            file: None,
            edited: false,
            open: false,
        };
        let meta = match store
            .read(&format!("projectMeta.{id}"))
            .map_err(library_failure)?
        {
            Some(meta) => Some(meta),
            None => store
                .read(&format!("projectState.{id}"))
                .map_err(library_failure)?,
        };
        row.file = meta
            .as_ref()
            .and_then(|m| m.get("file"))
            .and_then(|f| f.get("path"))
            .and_then(Value::as_str)
            .map(PathBuf::from);
        if let Some(published) = self.kernel.published(&id) {
            row.open = published.session.is_some();
            row.edited = published.dirty;
            row.file = published.path.clone().or(row.file);
            if let Some(root) = published.snapshot.root() {
                let v = open_summary(root);
                if let Some(name) = v.get("name").and_then(Value::as_str) {
                    row.name = name.into();
                }
                let summary = row.summary.as_mut().expect("summary");
                summary.width = dimension(&v, "w").or(summary.width);
                summary.height = dimension(&v, "h").or(summary.height);
                summary.fps = number(&v, "fps").or(summary.fps);
                summary.duration_seconds = number(&v, "dur").or(summary.duration_seconds);
            }
        }
        // A backing file inside the profile (a duplicate's recovery copy) is
        // the library's own, not a file the person saved: not shown, not revealed.
        if row
            .file
            .as_ref()
            .is_some_and(|f| f.starts_with(&self.profile))
        {
            row.file = None;
        }
        Ok(row)
    }
}
/// The realized project's `name`, `w`, `h`, `fps` and `dur` (the open
/// composition's), read in place rather than realizing the whole document.
fn open_summary(root: &pm_model::Node) -> Value {
    let comp = root
        .get("compId")
        .and_then(|id| match id.kind() {
            pm_model::NodeKind::String(s) => s.as_str(),
            _ => None,
        })
        .and_then(|id| root.get("comps")?.get(id));
    let mut m = pm_model::json::Map::new();
    if let Some(name) = root.get("name") {
        m.insert("name", name.to_value());
    }
    for k in ["w", "h", "fps", "dur"] {
        if let Some(v) = comp.and_then(|c| c.get(k)).or_else(|| root.get(k)) {
            m.insert(k, v.to_value());
        }
    }
    Value::Object(m)
}
fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.0)
}
fn dimension(v: &Value, key: &str) -> Option<u32> {
    number(v, key)
        .filter(|n| n.fract() == 0.0 && *n <= u32::MAX as f64)
        .map(|n| n as u32)
}
fn validate_id(id: &str) -> Result<(), Failure> {
    if (1..=120).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(())
    } else {
        Err(Failure::Service(ServiceErrorCode::InvalidInput))
    }
}
fn changed(reply: ShellReply) -> Result<(), Failure> {
    if matches!(reply, ShellReply::Changed(true)) {
        Ok(())
    } else {
        Err(Failure::Stale)
    }
}
fn kernel_failure(failure: KernelFailure) -> Failure {
    // KernelFailure::Storage carries diagnostic text only. Never parse it or
    // leak it to product copy. The kernel owner must add typed IO classification.
    let detail = failure.to_string();
    let code = match failure {
        KernelFailure::NoProject(_) => ServiceErrorCode::NotFound,
        _ => ServiceErrorCode::Unavailable,
    };
    Failure::Kernel { code, detail }
}
fn library_failure(error: LibraryError) -> Failure {
    Failure::Service(match error {
        LibraryError::Io(e) => match e.kind() {
            std::io::ErrorKind::NotFound => ServiceErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ServiceErrorCode::PermissionDenied,
            std::io::ErrorKind::StorageFull => ServiceErrorCode::DiskFull,
            _ => ServiceErrorCode::Unavailable,
        },
        LibraryError::Json { .. } => ServiceErrorCode::InvalidData,
        LibraryError::InvalidKey(_) => ServiceErrorCode::InvalidInput,
    })
}
pub(crate) fn project_value(id: &str, values: &NewProject) -> Result<Value, Failure> {
    let format = &values.format;
    // Validate again at the drivable service boundary, not just in the form.
    if values.name.trim().is_empty()
        || !(16..=8192).contains(&format.width)
        || !(16..=8192).contains(&format.height)
        || !format.fps.is_finite()
        || !(1.0..=240.0).contains(&format.fps)
        || !format.duration.is_finite()
        || !(0.1..=3600.0).contains(&format.duration)
        || !valid_colour(&format.background)
    {
        return Err(Failure::Service(ServiceErrorCode::InvalidInput));
    }
    // Use the existing JSON bridge and canonical project shape. No float
    // coercion or extension/document rewriting at this presentation boundary.
    let value = serde_json::json!({
        "id":id,"compId":pm_kernel::new_id("C"),"compName":"Main","comps":{},
        "name":values.name.trim(),"w":format.width,"h":format.height,"fps":format.fps,
        "dur":format.duration,"work":[0,format.duration],"bg":format.background,
        "assets":{},"layers":[],"params":{},"markers":[],
    });
    pm_model::json::parse(&value.to_string())
        .map_err(|_| Failure::Service(ServiceErrorCode::InvalidData))
}

fn valid_colour(value: &str) -> bool {
    matches!(value.len(), 7 | 9)
        && value.starts_with('#')
        && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}
