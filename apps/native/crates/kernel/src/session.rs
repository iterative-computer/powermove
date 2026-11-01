//! Small shell surface. Drive it through KernelHandle::shell (async).
use crate::{
    Kernel, KernelFailure, OpenInfo, OpenSource,
    journal::{Recovered, Recovery},
    library::LibraryStore,
    locks::{self, CopyReason, FileLock, OpenDecision},
    persistence::{DiskFormat, LibraryWriter},
    project_io::{self, OpenOptions, SaveOptions, SaveTarget},
};
use pm_model::json::{Map, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHandle {
    pub project: String,
    pub generation: String,
}
#[derive(Debug)]
pub enum SessionSource {
    Library { profile: PathBuf, project: String },
    File(PathBuf),
    New(Value),
}
#[derive(Debug)]
pub enum ShellCall {
    AttachLibrary {
        profile: PathBuf,
        format: DiskFormat,
        electron_profile: Option<PathBuf>,
    },
    Open {
        id: String,
        source: SessionSource,
        electron_profile: Option<PathBuf>,
    },
    Save(SessionHandle),
    SaveAs {
        session: SessionHandle,
        path: PathBuf,
    },
    Close(SessionHandle),
    Recent,
    CompactAll,
    Trash(String),
    Restore(String),
    Duplicate {
        project: String,
        id: String,
        name: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryResult {
    None,
    Recovered { truncated: bool },
    OfferCopy { project: String, truncated: bool },
    Unrecoverable { reason: String },
}
#[derive(Clone, Debug)]
pub enum ShellReply {
    Library(OpenDecision),
    Opened {
        session: SessionHandle,
        info: OpenInfo,
        decision: OpenDecision,
        recovery: RecoveryResult,
    },
    Compacted {
        projects: usize,
        recovery_copies: Vec<String>,
    },
    Saved {
        session: SessionHandle,
        recovery_copy: Option<String>,
    },
    Closed {
        recovery_copy: Option<String>,
    },
    Recent(Vec<Value>),
    Changed(bool),
}
pub(crate) enum Destination {
    Library(String),
    File(PathBuf),
}
pub(crate) struct Session {
    pub handle: SessionHandle,
    pub destination: Destination,
    pub lock: Option<FileLock>,
    pub meta: Value,
    pub rejected_history: Option<Value>,
}
fn failure(e: impl std::fmt::Display) -> KernelFailure {
    KernelFailure::Storage(e.to_string())
}
fn realize(k: &Kernel, id: &str) -> Result<Value, KernelFailure> {
    let store = k
        .project(id)
        .ok_or_else(|| KernelFailure::NoProject(id.into()))?
        .store();
    Ok(pm_model::canonical::realize_project(
        store.snapshot().document.to_value().unwrap_or(Value::Null),
    ))
}
fn with_identity(mut v: Value, id: &str, name: Option<&str>) -> Value {
    if let Value::Object(m) = &mut v {
        m.insert("id", Value::from(id));
        if let Some(name) = name {
            m.insert("name", Value::from(name));
        }
    }
    v
}
impl Kernel {
    pub fn shell(&mut self, call: ShellCall) -> Result<ShellReply, KernelFailure> {
        match call {
            ShellCall::AttachLibrary {
                profile,
                format,
                electron_profile,
            } => {
                if self.library.is_some() || !self.sessions.is_empty() {
                    return Err(failure("Library is already attached or sessions are open"));
                }
                // Beta never writes the source Electron profile, even after it quits.
                if electron_profile.as_ref().is_some_and(|p| {
                    p == &profile
                        || (p.canonicalize().ok().is_some()
                            && p.canonicalize().ok() == profile.canonicalize().ok())
                }) {
                    return Ok(ShellReply::Library(OpenDecision::AsCopy(
                        CopyReason::ForeignLibrary,
                    )));
                }
                match LibraryWriter::open(&profile, format, electron_profile.as_deref()) {
                    Ok(writer) => {
                        self.state =
                            crate::state::StateSnapshot::load(writer.store()).map_err(failure)?;
                        let record = profile.join("kernel-state.json");
                        match std::fs::read_to_string(&record) {
                            Ok(text) => {
                                let value = pm_model::json::parse(&text).map_err(failure)?;
                                let old = self.state.settings.clone();
                                self.state.restore_durable(&value).map_err(failure)?;
                                for key in old
                                    .keys()
                                    .chain(self.state.settings.keys())
                                    .collect::<std::collections::BTreeSet<_>>()
                                {
                                    match self.state.settings.get(key) {
                                        Some(v) => writer.set(key, v.clone()),
                                        None => writer.delete(key),
                                    }
                                    .map_err(failure)?;
                                }
                                writer.flush().map_err(failure)?;
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(failure(e)),
                        }
                        self.library = Some(writer);
                        Ok(ShellReply::Library(OpenDecision::Writable))
                    }
                    Err(decision) => Ok(ShellReply::Library(decision)),
                }
            }
            ShellCall::Open {
                id,
                source,
                electron_profile,
            } => self.open_session(id, source, electron_profile),
            ShellCall::Save(handle) => {
                self.check_session(&handle)?;
                self.save_session(&handle, None)
            }
            ShellCall::SaveAs { session, path } => {
                self.check_session(&session)?;
                self.save_session(&session, Some(path))
            }
            ShellCall::Close(handle) => {
                self.check_session(&handle)?;
                let saved = self.save_session(&handle, None)?;
                let ShellReply::Saved { recovery_copy, .. } = saved else {
                    unreachable!()
                };
                self.sessions.remove(&handle.project);
                self.close(&handle.project);
                Ok(ShellReply::Closed { recovery_copy })
            }
            ShellCall::CompactAll => {
                self.compacting = true;
                let result = self.compact_all();
                self.compacting = false;
                result
            }
            ShellCall::Recent => {
                let w = self
                    .library
                    .as_ref()
                    .ok_or_else(|| failure("Attach a native library first"))?;
                let mut rows = w.rows("projects").map_err(failure)?;
                rows.sort_by(|a, b| {
                    b.get("at")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                        .total_cmp(&a.get("at").and_then(Value::as_f64).unwrap_or(0.0))
                });
                Ok(ShellReply::Recent(rows))
            }
            ShellCall::Trash(id) => {
                if self.sessions.values().any(|s| {
                    s.handle.project == id
                        || matches!(&s.destination, Destination::Library(p) if p == &id)
                }) {
                    return Err(failure("Close the project before moving it to trash"));
                }
                Ok(ShellReply::Changed(
                    self.library
                        .as_ref()
                        .ok_or_else(|| failure("No library"))?
                        .trash(&id)
                        .map_err(failure)?,
                ))
            }
            ShellCall::Restore(id) => Ok(ShellReply::Changed(
                self.library
                    .as_ref()
                    .ok_or_else(|| failure("No library"))?
                    .restore(&id)
                    .map_err(failure)?,
            )),
            ShellCall::Duplicate { project, id, name } => {
                let w = self.library.as_ref().ok_or_else(|| failure("No library"))?;
                if w.store().load(&id).map_err(failure)?.is_some() {
                    return Err(failure("Copy id already exists"));
                }
                if let Some((open_id, session)) = self.sessions.iter().find(|(open_id, s)| {
                    *open_id == &project
                        || matches!(&s.destination, Destination::Library(slot) if slot == &project)
                }) {
                    let p = self.project(open_id).unwrap();
                    let copy = crate::library::LibraryProject {
                        id: id.clone(),
                        project: realize(self, open_id)?,
                        history: Some(p.store().history().snapshot().to_saved_packed()),
                        workspace: Some(p.workspace().clone()),
                        session: Some(
                            self.state
                                .ui
                                .get(open_id)
                                .map(|u| u.to_session())
                                .unwrap_or_else(|| session.meta.clone()),
                        ),
                        journal_applied: 0,
                        journal_skipped: 0,
                        from_autosave: false,
                    };
                    self.duplicate_slot(&id, &name, copy, Some(p.store().file().clone()))?;
                    return Ok(ShellReply::Changed(true));
                }
                let p = w
                    .store()
                    .load(&project)
                    .map_err(failure)?
                    .ok_or_else(|| failure("Missing project"))?;
                let associated = p
                    .session
                    .as_ref()
                    .and_then(|m| m.get("file"))
                    .and_then(|f| f.get("path"))
                    .and_then(Value::as_str)
                    .and_then(|path| {
                        project_io::open_project(std::path::Path::new(path), OpenOptions::default())
                            .ok()
                    })
                    .map(|o| o.file);
                self.duplicate_slot(&id, &name, p, associated)?;
                Ok(ShellReply::Changed(true))
            }
        }
    }
    fn duplicate_slot(
        &self,
        id: &str,
        name: &str,
        p: crate::library::LibraryProject,
        file: Option<project_io::ProjectFile>,
    ) -> Result<(), KernelFailure> {
        let w = self.library.as_ref().ok_or_else(|| failure("No library"))?;
        let mut meta = p.session.unwrap_or(Value::Null);
        let ws = p.workspace.unwrap_or(Value::Null);
        let history = p.history.unwrap_or(Value::Null);
        if let Some(file) = file.filter(|f| f.media.len() > 0) {
            let canonical = crate::model::to_canonical(p.project.clone());
            let comp = canonical.get("compId").and_then(Value::as_str);
            let imported = crate::History::import(&history, comp).unwrap_or_default();
            let mut store =
                crate::DocumentStore::adopt(pm_model::Document::new(Some(canonical), 1), imported);
            store.inherit_file(file);
            store.set_workspace(Some(ws.clone()));
            let path = w
                .store()
                .dir()
                .parent()
                .unwrap()
                .join("recovery-files")
                .join(format!("{id}.pmv"));
            std::fs::create_dir_all(path.parent().unwrap()).map_err(failure)?;
            let receipt = store
                .prepare_save(
                    SaveTarget::SaveAs {
                        path,
                        replace: false,
                    },
                    SaveOptions::default(),
                )
                .map_err(failure)?
                .run()
                .map_err(failure)?;
            store.finish_save(receipt).map_err(failure)?;
            meta = file_metadata(meta, &store);
        }
        w.save_project(
            id,
            with_identity(p.project, id, Some(name)),
            history,
            ws,
            meta,
        )
        .map_err(failure)
    }
    fn compact_all(&mut self) -> Result<ShellReply, KernelFailure> {
        let handles: Vec<_> = self.sessions.values().map(|s| s.handle.clone()).collect();
        let mut projects = 0;
        let mut recovery_copies = Vec::new();
        for handle in handles {
            if let ShellReply::Saved { recovery_copy, .. } = self.shell(ShellCall::Save(handle))? {
                projects += 1;
                recovery_copies.extend(recovery_copy);
            }
        }
        let Some(root) = &self.config_journal_root() else {
            return Ok(ShellReply::Compacted {
                projects,
                recovery_copies,
            });
        };
        if !root.exists() {
            return Ok(ShellReply::Compacted {
                projects,
                recovery_copies,
            });
        }
        let entries = std::fs::read_dir(root)
            .map_err(failure)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failure)?;
        let ids: Vec<_> = entries
            .into_iter()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        for id in ids {
            if crate::journal::sessions(&root.join(&id)).is_empty() {
                continue;
            }
            if self.sessions.values().any(|s| {
                s.handle.project == id
                    || matches!(&s.destination, Destination::Library(slot) if slot == &id)
            }) {
                continue;
            }
            let p = self
                .library
                .as_ref()
                .ok_or_else(|| failure("Attach a native library before compaction"))?
                .store()
                .load(&id)
                .map_err(failure)?;
            let Some(p) = p else {
                // Untitled/crashed work without a slot: materialize a recovery copy.
                match self.recover(&id, None, None) {
                    Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } => {
                        recovery_copies.push(self.recovery_copy(r, None)?);
                        projects += 1;
                    }
                    Recovery::Nothing => {}
                    Recovery::Unrecoverable { reason, .. } => return Err(failure(reason)),
                }
                continue;
            };
            let file = p
                .session
                .as_ref()
                .and_then(|s| s.get("file"))
                .and_then(|f| f.get("path"))
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let source = match file {
                Some(path) if path.is_file() => SessionSource::File(path),
                _ => SessionSource::Library {
                    profile: self.library.as_ref().unwrap().store().dir().to_path_buf(),
                    project: id.clone(),
                },
            };
            let ShellReply::Opened {
                session, recovery, ..
            } = self.open_session(id, source, None)?
            else {
                unreachable!()
            };
            if let RecoveryResult::OfferCopy { project, .. } = recovery {
                recovery_copies.push(project);
            }
            if let Destination::Library(slot) = &self.sessions[&session.project].destination {
                if slot != &session.project {
                    recovery_copies.push(slot.clone());
                }
            }
            let saved = self.shell(ShellCall::Close(session))?;
            if let ShellReply::Closed { recovery_copy } = saved {
                recovery_copies.extend(recovery_copy);
            }
            projects += 1;
        }
        Ok(ShellReply::Compacted {
            projects,
            recovery_copies,
        })
    }
    fn check_session(&self, handle: &SessionHandle) -> Result<(), KernelFailure> {
        if self
            .sessions
            .get(&handle.project)
            .is_some_and(|s| &s.handle == handle)
        {
            Ok(())
        } else {
            Err(failure("Session is closed or has been replaced"))
        }
    }
    fn open_session(
        &mut self,
        id: String,
        source: SessionSource,
        electron_profile: Option<PathBuf>,
    ) -> Result<ShellReply, KernelFailure> {
        if crate::library::parse_key(&format!("project.{id}")).is_none() {
            return Err(failure("Invalid project id"));
        }
        if self.project(&id).is_some() {
            return Err(KernelFailure::AlreadyOpen(id));
        }
        let mut decision = OpenDecision::Writable;
        let mut lock = None;
        let mut meta = Value::Object(Map::new());
        let mut rejected_history = None;
        let (mut source, mut destination, base, base_value) = match source {
            SessionSource::New(value) => {
                let w = self
                    .library
                    .as_ref()
                    .ok_or_else(|| failure("Attach a native library first"))?;
                if w.store().load(&id).map_err(failure)?.is_some() {
                    return Err(failure("Project id already exists"));
                }
                let value = with_identity(value, &id, None);
                let hash = crate::kernel::base_hash_of(&value);
                (
                    OpenSource::Untitled(value.clone()),
                    Destination::Library(id.clone()),
                    hash,
                    value,
                )
            }
            SessionSource::Library { profile, project } => {
                if let Some(s) = self.sessions.values().find(
                    |s| matches!(&s.destination, Destination::Library(slot) if slot == &project),
                ) {
                    return Err(KernelFailure::AlreadyOpen(s.handle.project.clone()));
                }
                let reader = LibraryStore::open(&profile);
                let p = reader
                    .load(&project)
                    .map_err(failure)?
                    .ok_or_else(|| failure("Library project is missing"))?;
                let owned = self.library.as_ref().is_some_and(|w| {
                    w.store().dir().canonicalize().ok() == reader.dir().canonicalize().ok()
                });
                decision = if owned {
                    OpenDecision::Writable
                } else {
                    OpenDecision::AsCopy(CopyReason::ForeignLibrary)
                };
                let policy = locks::open_decision(&profile, electron_profile.as_deref());
                if policy != OpenDecision::Writable {
                    decision = policy;
                }
                if p.session
                    .as_ref()
                    .and_then(|m| m.get("file"))
                    .and_then(|f| f.get("path"))
                    .and_then(Value::as_str)
                    .is_some_and(|p| !std::path::Path::new(p).is_file())
                {
                    decision = OpenDecision::AsCopy(CopyReason::OfflineFile);
                }
                meta = p.session.clone().unwrap_or(meta);
                rejected_history = p.history.clone();
                let hash = crate::kernel::base_hash_of(&p.project);
                let value = p.project.clone();
                let associated = p
                    .session
                    .as_ref()
                    .and_then(|m| m.get("file"))
                    .and_then(|m| m.get("path"))
                    .and_then(Value::as_str)
                    .and_then(|path| {
                        project_io::open_project(std::path::Path::new(path), OpenOptions::default())
                            .ok()
                    })
                    .map(|o| o.file);
                let mut target = Destination::Library(project.clone());
                if owned
                    && decision == OpenDecision::Writable
                    && let Some(file) = &associated
                    && let Some(path) = &file.path
                {
                    let backing = self
                        .library
                        .as_ref()
                        .unwrap()
                        .store()
                        .dir()
                        .parent()
                        .unwrap()
                        .join("recovery-files");
                    if !path.starts_with(backing) {
                        if let Some(s) = self
                            .sessions
                            .values()
                            .find(|s| matches!(&s.destination, Destination::File(p) if p == path))
                        {
                            return Err(KernelFailure::AlreadyOpen(s.handle.project.clone()));
                        }
                        let saved_hash = p
                            .session
                            .as_ref()
                            .and_then(|m| m.get("file"))
                            .and_then(|f| f.get("savedHash"))
                            .and_then(Value::as_str);
                        if saved_hash
                            .is_some_and(|h| file.version.as_ref().is_some_and(|v| v.hash != h))
                        {
                            decision = OpenDecision::AsCopy(CopyReason::ChangedOnDisk);
                        } else {
                            decision = locks::open_decision(path, electron_profile.as_deref());
                            if decision == OpenDecision::Writable {
                                match FileLock::acquire(&locks::project_lock_path(path)) {
                                    Ok(l) => {
                                        lock = Some(l);
                                        target = Destination::File(path.clone());
                                    }
                                    Err(_) => decision = OpenDecision::AsCopy(CopyReason::Locked),
                                }
                            }
                        }
                    }
                }
                let source = match associated {
                    Some(file) => OpenSource::LibraryWithFile { lib: p, file },
                    None => OpenSource::Library(p),
                };
                (source, target, hash, value)
            }

            SessionSource::File(path) => {
                if let Some(s) = self.sessions.values().find(|s| matches!(&s.destination, Destination::File(p) if p.canonicalize().ok().is_some() && p.canonicalize().ok() == path.canonicalize().ok())) { return Err(KernelFailure::AlreadyOpen(s.handle.project.clone())); }
                if let Some(w) = &self.library {
                    if let Some(existing) = w.store().load(&id).map_err(failure)? {
                        let associated = existing
                            .session
                            .as_ref()
                            .and_then(|m| m.get("file"))
                            .and_then(|m| m.get("path"))
                            .and_then(Value::as_str);
                        if associated != path.to_str() {
                            return Err(failure(
                                "Project id belongs to another library project; open with a fresh session id",
                            ));
                        }
                    }
                }
                decision = locks::open_decision(&path, electron_profile.as_deref());
                if decision == OpenDecision::Writable {
                    match FileLock::acquire(&locks::project_lock_path(&path)) {
                        Ok(l) => lock = Some(l),
                        Err(e) => {
                            decision = OpenDecision::AsCopy(
                                if e.kind() == std::io::ErrorKind::WouldBlock {
                                    CopyReason::Locked
                                } else {
                                    CopyReason::UncertainLock
                                },
                            )
                        }
                    }
                }
                let opened =
                    project_io::open_project(&path, OpenOptions::default()).map_err(failure)?;
                let value = opened.document.to_value().unwrap_or(Value::Null);
                let hash = crate::kernel::base_hash_of(&value);
                (
                    OpenSource::File(opened),
                    Destination::File(path),
                    hash,
                    value,
                )
            }
        };
        let recovery_file = match &source {
            OpenSource::File(o) => Some(o.file.clone()),
            OpenSource::LibraryWithFile { file, .. } => Some(file.clone()),
            _ => None,
        };
        let recovery_id = match &destination {
            Destination::Library(slot) => slot.as_str(),
            _ => id.as_str(),
        };
        let recovery = match self.recover(
            recovery_id,
            Some(&base),
            Some(&crate::model::to_canonical(base_value)),
        ) {
            Recovery::Nothing => RecoveryResult::None,
            Recovery::Unrecoverable { reason, .. } => RecoveryResult::Unrecoverable { reason },
            Recovery::Copy { recovered, .. } => {
                let truncated = recovered.truncated;
                let copy = self.recovery_copy(recovered, recovery_file.clone())?;
                RecoveryResult::OfferCopy {
                    project: copy,
                    truncated,
                }
            }
            Recovery::Recovered(recovered) => {
                let truncated = recovered.truncated;
                source = match source {
                    OpenSource::LibraryWithFile { mut file, .. } => {
                        file.envelope.workspace = Some(recovered.workspace);
                        OpenSource::RecoveredFile {
                            opened: project_io::OpenedProject {
                                file,
                                document: recovered.document,
                                open_comp: None,
                                history: Some(recovered.history),
                            },
                            supersedes: vec![recovered.session],
                        }
                    }
                    OpenSource::File(mut opened) => {
                        opened.document = recovered.document;
                        opened.history = Some(recovered.history);
                        opened.file.envelope.workspace = Some(recovered.workspace);
                        OpenSource::RecoveredFile {
                            opened,
                            supersedes: vec![recovered.session],
                        }
                    }
                    _ => OpenSource::Recovered {
                        supersedes: vec![recovered.session.clone()],
                        recovered,
                    },
                };
                RecoveryResult::Recovered { truncated }
            }
        };
        if decision != OpenDecision::Writable {
            // Detach saves from the original. Copies get unique library identities.
            destination = Destination::Library(crate::new_id("copy"));
            lock = None;
            if self.library.is_none() {
                return Err(failure(
                    "Attach a writable native library to open as a copy",
                ));
            }
        }
        let mut info = self.open(&id, source)?;
        let ui = crate::state::UiState::from_session(meta.clone());
        std::sync::Arc::make_mut(&mut self.state.ui).insert(id.clone(), ui);
        if !info.history_rejected {
            rejected_history = None;
        }
        let handle = SessionHandle {
            project: id.clone(),
            generation: crate::new_id("session"),
        };
        self.sessions.insert(
            id,
            Session {
                handle: handle.clone(),
                destination,
                lock,
                meta,
                rejected_history,
            },
        );
        // A new/copy project gets a rollback-readable slot immediately.
        if matches!(
            self.sessions[&handle.project].destination,
            Destination::Library(_)
        ) {
            if let Err(e) = self.save_session(&handle, None) {
                self.sessions.remove(&handle.project);
                self.close(&handle.project);
                return Err(e);
            }
            info.journal = self.journal_dir(&handle.project);
            info.base_hash = self.base_hash(&handle.project).unwrap().into();
        } else if self.library.is_some() {
            if let Err(e) = self.save_library_slot(&handle.project, &handle.project, false) {
                self.sessions.remove(&handle.project);
                self.close(&handle.project);
                return Err(e);
            }
        }
        Ok(ShellReply::Opened {
            session: handle,
            info,
            decision,
            recovery,
        })
    }
    fn recovery_copy(
        &self,
        recovered: Recovered,
        file: Option<project_io::ProjectFile>,
    ) -> Result<String, KernelFailure> {
        let w = self
            .library
            .as_ref()
            .ok_or_else(|| failure("Attach a library to preserve recovery work"))?;
        let id = crate::new_id("recovery");
        let v = pm_model::canonical::realize_project(
            recovered.document.to_value().unwrap_or(Value::Null),
        );
        let name = format!(
            "{} — Recovered",
            v.get("name").and_then(Value::as_str).unwrap_or("Project")
        );
        let history = recovered.history.snapshot().to_saved_packed();
        let workspace = recovered.workspace.clone();
        let mut meta = Value::Object(Map::new());
        if let Some(file) = file.filter(|f| f.media.len() > 0) {
            let mut store = crate::DocumentStore::recovered(recovered.document, recovered.history);
            store.inherit_file(file);
            store.set_workspace(Some(workspace.clone()));
            let path = w
                .store()
                .dir()
                .parent()
                .unwrap()
                .join("recovery-files")
                .join(format!("{id}.pmv"));
            std::fs::create_dir_all(path.parent().unwrap()).map_err(failure)?;
            let receipt = store
                .prepare_save(
                    SaveTarget::SaveAs {
                        path: path.clone(),
                        replace: false,
                    },
                    SaveOptions::default(),
                )
                .map_err(failure)?
                .run()
                .map_err(failure)?;
            store.finish_save(receipt).map_err(failure)?;
            meta = file_metadata(meta, &store);
        }
        w.save_project(
            &id,
            with_identity(v, &id, Some(&name)),
            history,
            workspace,
            meta,
        )
        .map_err(failure)?;
        Ok(id)
    }
    fn save_session(
        &mut self,
        handle: &SessionHandle,
        save_as: Option<PathBuf>,
    ) -> Result<ShellReply, KernelFailure> {
        let id = &handle.project;
        let mut new_lock = None;
        if let Some(path) = &save_as {
            let decision = locks::open_decision(path, None);
            if decision != OpenDecision::Writable {
                return Err(failure(format!(
                    "Choose a writable local destination: {decision:?}"
                )));
            }
            new_lock = Some(FileLock::acquire(&locks::project_lock_path(path)).map_err(failure)?);
        }
        let mut recovery_copy = None;
        let file_path = save_as
            .clone()
            .or_else(|| match &self.sessions[id].destination {
                Destination::File(path) => Some(path.clone()),
                _ => None,
            });
        if let Some(path) = file_path {
            let ws = self.project(id).unwrap().workspace().clone();
            let p = self.project_mut(id).unwrap();
            p.store_mut().set_workspace(Some(ws));
            let target = if save_as.is_some() {
                SaveTarget::SaveAs {
                    path: path.clone(),
                    replace: false,
                }
            } else {
                SaveTarget::InPlace
            };
            let result = p
                .store_mut()
                .prepare_save(target, SaveOptions::default())
                .and_then(|plan| plan.run())
                .and_then(|receipt| p.store_mut().finish_save(receipt));
            if let Err(e) = result {
                if save_as.is_some() {
                    return Err(failure(e));
                }
                let copy = crate::new_id("recovery");
                self.save_library_slot(id, &copy, true)?;
                recovery_copy = Some(copy.clone());
                self.sessions.get_mut(id).unwrap().destination = Destination::Library(copy);
                self.sessions.get_mut(id).unwrap().lock = None;
            } else if save_as.is_some() {
                let s = self.sessions.get_mut(id).unwrap();
                s.destination = Destination::File(path);
                s.lock = new_lock;
            }
        } else {
            let Destination::Library(slot) = &self.sessions[id].destination else {
                unreachable!()
            };
            let slot = slot.clone();
            self.save_library_slot(id, &slot, false)?;
        }
        if matches!(self.sessions[id].destination, Destination::File(_)) && self.library.is_some() {
            self.save_library_slot(id, id, false)?;
        }
        self.rebase_journal(id)?;
        Ok(ShellReply::Saved {
            session: handle.clone(),
            recovery_copy,
        })
    }
    fn save_library_slot(
        &mut self,
        id: &str,
        slot: &str,
        recovery: bool,
    ) -> Result<(), KernelFailure> {
        let mut value = realize(self, id)?;
        let name = if recovery {
            Some(format!(
                "{} — Recovered",
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Project")
            ))
        } else if value.get("id").and_then(Value::as_str) != Some(slot) {
            Some(format!(
                "{} — Native",
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Project")
            ))
        } else {
            None
        };
        value = with_identity(value, slot, name.as_deref());
        if self.project(id).unwrap().store().media().len() > 0
            && (recovery || matches!(self.sessions[id].destination, Destination::Library(_)))
        {
            let w = self
                .library
                .as_ref()
                .ok_or_else(|| failure("No native library"))?;
            let path = w
                .store()
                .dir()
                .parent()
                .unwrap()
                .join("recovery-files")
                .join(format!("{slot}.pmv"));
            std::fs::create_dir_all(path.parent().unwrap()).map_err(failure)?;
            let ws = self.project(id).unwrap().workspace().clone();
            let p = self.project_mut(id).unwrap();
            p.store_mut().set_workspace(Some(ws));
            let target = if p.store().path() == Some(path.as_path()) {
                SaveTarget::InPlace
            } else {
                SaveTarget::SaveAs {
                    path,
                    replace: false,
                }
            };
            let receipt = p
                .store_mut()
                .prepare_save(target, SaveOptions::default())
                .map_err(failure)?
                .run()
                .map_err(failure)?;
            p.store_mut().finish_save(receipt).map_err(failure)?;
        }
        let p = self.project(id).unwrap();
        let session = &self.sessions[id];
        let history = session
            .rejected_history
            .clone()
            .filter(|_| p.store().history().len() == 0)
            .unwrap_or_else(|| p.store().history().snapshot().to_saved_packed());
        let mut meta = self
            .state
            .ui
            .get(id)
            .map(|ui| ui.to_session())
            .unwrap_or_else(|| session.meta.clone());
        if p.store().path().is_some() {
            meta = file_metadata(meta, p.store());
        }
        let writer = self
            .library
            .as_ref()
            .ok_or_else(|| failure("Attach a native library first"))?;
        let result = if self.compacting {
            writer.compact_project(slot, value, history, p.workspace().clone(), meta)
        } else {
            writer.save_project(slot, value, history, p.workspace().clone(), meta)
        };
        result.map_err(failure)?;
        self.project_mut(id)
            .unwrap()
            .store_mut()
            .mark_library_saved();
        Ok(())
    }
}

fn file_metadata(meta: Value, store: &crate::DocumentStore) -> Value {
    let mut m = meta.as_object().cloned().unwrap_or_default();
    let mut file = m
        .get("file")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(path) = store.path() {
        file.insert("path", Value::from(path.to_string_lossy().into_owned()));
    }
    if let Some(version) = &store.file().version {
        file.insert("savedHash", Value::from(version.hash.clone()));
        file.insert("baselineHash", Value::from(version.hash.clone()));
    }
    m.insert("file", Value::Object(file));
    Value::Object(m)
}
