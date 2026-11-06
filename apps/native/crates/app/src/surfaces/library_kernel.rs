//! Worker adapter. The kernel remains the only writer; LibraryStore is read-only.
use super::{controller::*, form::NewProject, model::*};
use pm_kernel::{
    KernelFailure, KernelHandle,
    library::{LibraryError, LibraryStore},
    session::{SessionSource, ShellCall, ShellReply},
};
use pm_model::json::Value;
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
            mpsc::TrySendError::Disconnected(_) => Failure::Service(ErrorCode::Unavailable),
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
                _ => Err(Failure::Service(ErrorCode::Unavailable)),
            };
        }
        let plan = self
            .kernel
            .prepare_save(session, None)
            .wait()
            .ok_or(Failure::Service(ErrorCode::Unavailable))?
            .map_err(kernel_failure)?;
        // File packing and IO execute on this worker, outside the actor.
        let completion = plan.run();
        match self
            .kernel
            .finish_save(completion)
            .wait()
            .ok_or(Failure::Service(ErrorCode::Unavailable))?
            .map_err(kernel_failure)?
        {
            ShellReply::Saved { .. } => Ok(()),
            _ => Err(Failure::Service(ErrorCode::Unavailable)),
        }
    }
    fn shell(&self, call: ShellCall) -> Result<ShellReply, Failure> {
        self.kernel
            .shell(call)
            .wait()
            .ok_or(Failure::Service(ErrorCode::Unavailable))?
            .map_err(kernel_failure)
    }
    pub fn execute(&mut self, command: Command) -> Result<Outcome, Failure> {
        match command {
            Command::Load(query) => {
                let snapshot = self.snapshot()?;
                let rows = query.apply_with_names(&snapshot, compare_names);
                Ok(Outcome::Loaded { snapshot, rows })
            }
            Command::Open { id, new_window } => {
                validate_id(&id)?;
                if self
                    .kernel
                    .published(&id)
                    .is_some_and(|p| p.session.is_some())
                {
                    return Ok(Outcome::Focus(id));
                }
                let reply = self.shell(ShellCall::PrepareOpen {
                    id: id.clone(),
                    source: SessionSource::Library {
                        profile: self.profile.clone(),
                        project: id,
                    },
                    electron_profile: None,
                })?;
                match reply {
                    ShellReply::Opened { .. } | ShellReply::RecoveryRequired { .. } => {
                        Ok(Outcome::Opened { reply, new_window })
                    }
                    _ => Err(Failure::Service(ErrorCode::Unavailable)),
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
                    return Err(Failure::Service(ErrorCode::Unavailable));
                }
                Ok(Outcome::Opened {
                    reply,
                    new_window: false,
                })
            }
            Command::Duplicate { id, name } => {
                validate_id(&id)?;
                if name.trim().is_empty() {
                    return Err(Failure::Service(ErrorCode::InvalidInput));
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
                    return Err(Failure::Service(ErrorCode::InvalidInput));
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
                    .ok_or(Failure::Service(ErrorCode::Unavailable))?
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
                changed(self.shell(ShellCall::Trash(id))?)?;
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
            Command::PutBackAndSave(id) => {
                validate_id(&id)?;
                let session = self
                    .kernel
                    .published(&id)
                    .and_then(|p| p.session.as_ref().map(|s| s.handle.clone()))
                    .ok_or(Failure::Stale)?;
                changed(self.shell(ShellCall::Restore(id))?)?;
                self.save(session)?;
                Ok(Outcome::Changed)
            }
        }
    }
    pub fn snapshot(&self) -> Result<Snapshot, Failure> {
        let ShellReply::Recent(recent) = self.shell(ShellCall::Recent)? else {
            return Err(Failure::Service(ErrorCode::Unavailable));
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
            rows.push(self.row(&store, entry.raw, true)?);
        }
        Ok(Snapshot { rows, recents })
    }
    fn row(&self, store: &LibraryStore, raw: Value, trashed: bool) -> Result<Row, Failure> {
        let id = raw
            .get("id")
            .and_then(Value::as_str)
            .ok_or(Failure::Service(ErrorCode::InvalidData))?
            .to_owned();
        validate_id(&id)?;
        let name = raw
            .get("name")
            .and_then(Value::as_str)
            .ok_or(Failure::Service(ErrorCode::InvalidData))?
            .to_owned();
        let mut row = Row {
            modified_ms: raw
                .get("at")
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
            if let Some(document) = published.snapshot.document.to_value() {
                let v = pm_model::canonical::realize_project(document);
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
        Ok(row)
    }
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
        Err(Failure::Service(ErrorCode::InvalidInput))
    }
}
fn changed(reply: ShellReply) -> Result<(), Failure> {
    if matches!(reply, ShellReply::Changed(true)) {
        Ok(())
    } else {
        Err(Failure::Stale)
    }
}
fn kernel_failure(_: KernelFailure) -> Failure {
    // KernelFailure::Storage carries diagnostic text only. Never parse it or
    // leak it to product copy. The kernel owner must add typed IO classification.
    Failure::Service(ErrorCode::Unavailable)
}
fn library_failure(error: LibraryError) -> Failure {
    Failure::Service(match error {
        LibraryError::Io(e) => match e.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            std::io::ErrorKind::StorageFull => ErrorCode::DiskFull,
            _ => ErrorCode::Unavailable,
        },
        LibraryError::Json { .. } => ErrorCode::InvalidData,
        LibraryError::InvalidKey(_) => ErrorCode::InvalidInput,
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
        return Err(Failure::Service(ErrorCode::InvalidInput));
    }
    // Use the existing JSON bridge and canonical project shape. No float
    // coercion or extension/document rewriting at this presentation boundary.
    let value = serde_json::json!({
        "id":id,"compId":pm_kernel::new_id("C"),"compName":"Main","comps":{},
        "name":values.name.trim(),"w":format.width,"h":format.height,"fps":format.fps,
        "dur":format.duration,"work":[0,format.duration],"bg":format.background,
        "assets":{},"layers":[],"params":{},"markers":[],
    });
    pm_model::json::parse(&value.to_string()).map_err(|_| Failure::Service(ErrorCode::InvalidData))
}

fn valid_colour(value: &str) -> bool {
    matches!(value.len(), 7 | 9)
        && value.starts_with('#')
        && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSComparisonResult, NSString};
        match NSString::from_str(a).localizedCompare(&NSString::from_str(b)) {
            NSComparisonResult::Ascending => std::cmp::Ordering::Less,
            NSComparisonResult::Descending => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        a.to_lowercase().cmp(&b.to_lowercase())
    }
}
