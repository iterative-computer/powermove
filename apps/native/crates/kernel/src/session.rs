//! Small shell surface. Drive it through KernelHandle::shell (async).
use crate::{
    Kernel, KernelFailure, OpenInfo, OpenSource,
    journal::{Recovered, Recovery},
    library::LibraryStore,
    locks::{self, CopyReason, FileLock, OpenDecision},
    persistence::{DiskFormat, LibraryWriter},
    project_io::{self, OpenOptions, SaveTarget},
};
use pm_model::json::{Map, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHandle {
    pub project: String,
    pub generation: String,
}
#[derive(Clone, Debug)]
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
    /// Prepare without publishing recovered work. Resolve only this ticket.
    PrepareOpen {
        id: String,
        source: SessionSource,
        electron_profile: Option<PathBuf>,
    },
    DecideRecovery {
        ticket: String,
        decision: RecoveryDecision,
    },
    Save(SessionHandle),
    SaveAs {
        session: SessionHandle,
        path: PathBuf,
        /// The save panel asked; the displaced file is kept as a backup.
        replace: bool,
    },
    Close(SessionHandle),
    /// Leave journal recoverable, without saving the base.
    DeferClose(SessionHandle),
    /// Archive rejected work, release the session, and leave its base untouched.
    DiscardClose(SessionHandle),
    /// All earlier acknowledged writes and checkpoints are fsynced on success.
    DurabilityBarrier,
    /// Compaction and durability barrier; refuses any in-flight save/open entry.
    /// External files are never written: their unsaved work stays in the
    /// journal and the next open asks the person about it.
    PrepareQuit,
    /// The last call of a quit: flush and unlock the library (no open
    /// sessions may remain). Later library calls fail.
    ReleaseLibrary,
    AbortSave {
        session: SessionHandle,
        ticket: String,
    },
    Recent,
    /// Every journal into its library slot or file (the restore build's
    /// rollback step, plan §6).
    CompactAll,
    Trash(String),
    Restore(String),
    Duplicate {
        project: String,
        id: String,
        name: String,
    },
    /// The library's takes (v1's `takes` store), newest first.
    Takes,
    /// Replaces the takes only while they still equal `expected` (what the
    /// caller read), so a take saved elsewhere meanwhile is never lost.
    /// Replies `Changed(false)` when they no longer match.
    ReplaceTakes {
        expected: Vec<Value>,
        next: Vec<Value>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryDecision {
    Recover,
    OpenAsCopy,
    Discard,
    /// Only for an unrecoverable journal: it is kept, set aside, and the saved
    /// project opens. Recoverable edits can't be walked past with this.
    OpenSavedVersion,
}

#[derive(Clone, Debug)]
pub struct SessionMetadata {
    pub handle: SessionHandle,
    pub destination: SessionDestination,
    pub metadata: Value,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionDestination {
    Library(String),
    File(PathBuf),
}

pub(crate) struct PendingRecovery {
    id: String,
    source: SessionSource,
    electron_profile: Option<PathBuf>,
    stamp: String,
    /// What the prompt was prepared from, reused by the decision while
    /// `fresh` still describes the disk (so a large project isn't read twice).
    prepared: Option<Box<Prepared>>,
    fresh: Freshness,
}
/// An open read up to its recovery decision.
pub(crate) struct Prepared {
    id: String,
    source: OpenSource,
    destination: Destination,
    lock: Option<FileLock>,
    decision: OpenDecision,
    meta: Value,
    rejected_history: Option<Value>,
    base: String,
    recovered: Recovery,
    recovery_file: Option<project_io::ProjectFile>,
}
/// Sizes and modification times of the files an open read.
type Freshness = Vec<(PathBuf, Option<(u64, std::time::SystemTime)>)>;
fn freshness(paths: impl IntoIterator<Item = PathBuf>) -> Freshness {
    paths
        .into_iter()
        .map(|path| {
            let seen = std::fs::metadata(&path)
                .ok()
                .and_then(|m| Some((m.len(), m.modified().ok()?)));
            (path, seen)
        })
        .collect()
}
impl Prepared {
    /// The file, slot and journal files this preparation read.
    fn inputs(&self, journals: Option<PathBuf>, library: Option<&std::path::Path>) -> Vec<PathBuf> {
        let mut paths: Vec<_> = self
            .recovery_file
            .as_ref()
            .and_then(|f| f.path.clone())
            .into_iter()
            .collect();
        if let (Destination::Library(slot), Some(store)) = (&self.destination, library) {
            paths.push(store.join(format!("project.{slot}.json")));
        }
        for session in journals
            .map(|d| crate::journal::sessions(&d))
            .unwrap_or_default()
        {
            paths.push(session.clone());
            paths.extend(
                std::fs::read_dir(&session)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path()),
            );
        }
        paths.sort();
        paths
    }
}
enum RecoveryMode {
    Legacy,
    Ask,
    Decide(RecoveryDecision, String),
}
pub(crate) struct PendingSave {
    ticket: String,
    path: Option<PathBuf>,
    lock: Option<FileLock>,
    revision: u64,
}
/// Run on a worker, then send the completion to KernelHandle::finish_save.
/// Dropping a handoff requires AbortSave after its plan has been dropped.
pub struct SessionSave {
    pub session: SessionHandle,
    pub ticket: String,
    pub plan: crate::SavePlan,
}
pub struct SessionSaveCompletion {
    session: SessionHandle,
    ticket: String,
    result: Result<crate::SaveReceipt, crate::ProjectIoError>,
}
impl SessionSaveCompletion {
    pub(crate) fn project(&self) -> &str {
        &self.session.project
    }
}
impl SessionSave {
    pub fn run(self) -> SessionSaveCompletion {
        SessionSaveCompletion {
            session: self.session,
            ticket: self.ticket,
            result: self.plan.run(),
        }
    }
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
    RecoveryRequired {
        ticket: String,
        recovery: RecoveryResult,
    },
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
    Durable,
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
    pub pending_save: Option<PendingSave>,
}
fn failure(e: impl std::fmt::Display) -> KernelFailure {
    KernelFailure::Storage(e.to_string())
}
fn realize(k: &Kernel, id: &str) -> Result<Value, KernelFailure> {
    k.refuse_faulted(id)?;
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
/// The opened project's canonical document (the base recovery replays onto).
/// A file's is shared, not copied.
fn source_document(source: &OpenSource) -> pm_model::Document {
    let value = match source {
        OpenSource::File(opened) => return opened.document.clone(),
        OpenSource::Library(lib) | OpenSource::LibraryWithFile { lib, .. } => lib.project.clone(),
        OpenSource::Untitled(value) => value.clone(),
        _ => Value::Null,
    };
    pm_model::Document::new(Some(crate::model::to_canonical(value)), 1)
}
// A durable rebase checkpoint after a clean close contains no unsaved work.
// Compare history and workspace too; document equality alone would hide edits.
fn saved_state(source: &OpenSource) -> Option<(crate::History, Value)> {
    match source {
        OpenSource::File(opened) => Some((
            opened.history.clone().unwrap_or_default(),
            opened
                .file
                .envelope
                .workspace
                .clone()
                .unwrap_or(Value::Null),
        )),
        OpenSource::Library(lib) | OpenSource::LibraryWithFile { lib, .. } => {
            let canonical = crate::model::to_canonical(lib.project.clone());
            let history = lib.history.as_ref().and_then(|h| {
                crate::History::import(h, canonical.get("compId").and_then(Value::as_str)).ok()
            });
            Some((
                history.unwrap_or_default(),
                lib.workspace
                    .clone()
                    .or_else(|| {
                        lib.session
                            .as_ref()
                            .and_then(|s| s.get("workspace"))
                            .cloned()
                    })
                    .unwrap_or(Value::Null),
            ))
        }
        _ => None,
    }
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
            ShellCall::PrepareOpen {
                id,
                source,
                electron_profile,
            } => self.open_session_mode(id, source, electron_profile, RecoveryMode::Ask),
            ShellCall::DecideRecovery { ticket, decision } => {
                let mut pending = self
                    .pending_recovery
                    .remove(&ticket)
                    .ok_or_else(|| failure("Recovery choice expired; prepare the open again"))?;
                if let Some(prepared) = pending.prepared.take() {
                    let journals = self.project_dir(match &prepared.destination {
                        Destination::Library(slot) => slot,
                        _ => &prepared.id,
                    });
                    let library = self.library.as_ref().map(|w| w.store().dir().to_path_buf());
                    if freshness(prepared.inputs(journals, library.as_deref())) == pending.fresh {
                        return self.settle_open(*prepared, Some(decision));
                    }
                }
                self.open_session_mode(
                    pending.id,
                    pending.source,
                    pending.electron_profile,
                    RecoveryMode::Decide(decision, pending.stamp),
                )
            }
            ShellCall::AbortSave { session, ticket } => {
                self.check_session(&session)?;
                let s = &self.sessions[&session.project];
                if s.pending_save.as_ref().is_none_or(|p| p.ticket != ticket)
                    || self.project(&session.project).unwrap().store().is_saving()
                {
                    return Err(failure("Save is still in flight or ticket expired"));
                }
                self.sessions
                    .get_mut(&session.project)
                    .unwrap()
                    .pending_save = None;
                Ok(ShellReply::Changed(true))
            }
            ShellCall::DeferClose(handle) => {
                self.ensure_closable(&handle)?;
                self.flush_project(&handle.project)?;
                self.remove_closed_project(&handle.project);
                Ok(ShellReply::Closed {
                    recovery_copy: None,
                })
            }
            ShellCall::DiscardClose(handle) => {
                self.ensure_closable(&handle)?;
                self.archive_journal(&handle.project)?;
                self.remove_closed_project(&handle.project);
                Ok(ShellReply::Closed {
                    recovery_copy: None,
                })
            }
            ShellCall::DurabilityBarrier => {
                self.durability_barrier()?;
                Ok(ShellReply::Durable)
            }
            ShellCall::PrepareQuit => {
                // A stopped project is never compacted: its journal is the work.
                for session in self.sessions.values() {
                    if self.fault(&session.handle.project).is_none() {
                        self.ensure_idle(&session.handle)?;
                    }
                }
                self.compacting = true;
                let result = self.compact_all(false);
                self.compacting = false;
                let result = result?;
                self.durability_barrier()?;
                Ok(result)
            }
            ShellCall::ReleaseLibrary => {
                if !self.sessions.is_empty() {
                    return Err(failure("Close every project before releasing the library"));
                }
                if let Some(writer) = self.library.take() {
                    let flushed = writer.flush();
                    // Dropping the writer stops it and releases library.lock.
                    drop(writer);
                    flushed.map_err(failure)?;
                }
                Ok(ShellReply::Durable)
            }
            ShellCall::Save(handle) => {
                self.check_session(&handle)?;
                self.save_session(&handle, None)
            }
            ShellCall::SaveAs {
                session,
                path,
                replace,
            } => {
                self.check_session(&session)?;
                self.save_session(&session, Some((path, replace)))
            }
            ShellCall::Close(handle) if self.fault(&handle.project).is_some() => {
                // Nothing of a stopped project is written: its journal holds
                // every edit acknowledged before the error, and the next open
                // offers to recover it.
                self.ensure_closable(&handle)?;
                self.flush_project(&handle.project)?;
                self.remove_closed_project(&handle.project);
                Ok(ShellReply::Closed {
                    recovery_copy: None,
                })
            }
            ShellCall::Close(handle) => {
                self.ensure_idle(&handle)?;
                self.flush_project(&handle.project)?;
                // A file that already holds everything is not rewritten.
                let unchanged = matches!(
                    self.sessions[&handle.project].destination,
                    Destination::File(_)
                ) && !self.project(&handle.project).unwrap().store().needs_save();
                let recovery_copy = if unchanged {
                    self.save_session_meta(&handle.project)?;
                    None
                } else {
                    let saved = self.save_session(&handle, None)?;
                    let ShellReply::Saved { recovery_copy, .. } = saved else {
                        unreachable!()
                    };
                    recovery_copy
                };
                self.flush_project(&handle.project)?;
                self.durability_barrier()?;
                let journals = self.project_dir(&handle.project);
                self.remove_closed_project(&handle.project);
                // Saved where it belongs and durable: the journal holds nothing
                // more. Left behind, every quit reopened the project to compact
                // it, and a later open asked to recover work that was saved.
                if recovery_copy.is_none()
                    && let Some(dir) = journals
                {
                    for session in crate::journal::sessions(&dir) {
                        std::fs::remove_dir_all(session).map_err(failure)?;
                    }
                }
                Ok(ShellReply::Closed { recovery_copy })
            }
            ShellCall::CompactAll => {
                self.compacting = true;
                let result = self.compact_all(true);
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
            ShellCall::Takes => {
                let w = self.library.as_ref().ok_or_else(|| failure("No library"))?;
                Ok(ShellReply::Recent(w.rows("takes").map_err(failure)?))
            }
            ShellCall::ReplaceTakes { expected, next } => {
                let w = self.library.as_ref().ok_or_else(|| failure("No library"))?;
                if w.rows("takes").map_err(failure)? != expected {
                    return Ok(ShellReply::Changed(false));
                }
                w.set("takes", Value::Array(next)).map_err(failure)?;
                w.flush().map_err(failure)?;
                Ok(ShellReply::Changed(true))
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
                let p = match w.store().load(&project).map_err(failure)? {
                    Some(p) => p,
                    None => self.file_entry_project(&project)?,
                };
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
    pub fn session_metadata(&self, id: &str) -> Option<SessionMetadata> {
        let s = self.sessions.get(id)?;
        Some(SessionMetadata {
            handle: s.handle.clone(),
            metadata: s.meta.clone(),
            destination: match &s.destination {
                Destination::Library(id) => SessionDestination::Library(id.clone()),
                Destination::File(path) => SessionDestination::File(path.clone()),
            },
        })
    }
    /// [`ensure_idle`](Self::ensure_idle) for a close that saves nothing: a
    /// project stopped by an internal error closes even with an edit open.
    fn ensure_closable(&self, handle: &SessionHandle) -> Result<(), KernelFailure> {
        if self.fault(&handle.project).is_none() {
            return self.ensure_idle(handle);
        }
        self.check_session(handle)?;
        if self.project(&handle.project).unwrap().store().is_saving() {
            return Err(failure("A save is in flight; close again when it finishes"));
        }
        Ok(())
    }
    fn ensure_idle(&self, handle: &SessionHandle) -> Result<(), KernelFailure> {
        self.check_session(handle)?;
        self.refuse_faulted(&handle.project)?;
        let p = self.project(&handle.project).unwrap();
        if p.store().is_saving() || self.sessions[&handle.project].pending_save.is_some() {
            return Err(failure(
                "A save is in flight; finish it before saving or closing",
            ));
        }
        if p.open_entry().is_some() {
            return Err(failure("Finish the active edit before saving or closing"));
        }
        Ok(())
    }
    /// Freeze an independent packed export. It neither adopts a destination
    /// nor changes the live save/dirty/history state.
    pub fn prepare_session_export(
        &self,
        session: SessionHandle,
        document: u64,
        path: PathBuf,
    ) -> Result<crate::SavePlan, KernelFailure> {
        self.check_session(&session)?;
        self.refuse_faulted(&session.project)?;
        let project = self
            .project(&session.project)
            .ok_or_else(|| failure("Project closed"))?;
        if project.store().doc_id() != document {
            return Err(failure("Project changed before export"));
        }
        let mut plan = project
            .store()
            .prepare_portable_export(path)
            .map_err(failure)?;
        plan.workspace = Some(project.workspace().clone());
        Ok(plan)
    }

    /// Freeze a file save on the actor; the worker owns disk IO only.
    pub fn prepare_session_save(
        &mut self,
        session: SessionHandle,
        path: Option<PathBuf>,
        replace: bool,
    ) -> Result<SessionSave, KernelFailure> {
        self.ensure_idle(&session)?;
        let mut lock = None;
        if let Some(path) = &path {
            if locks::open_decision(path, None) != OpenDecision::Writable {
                return Err(failure("Choose a writable local destination"));
            }
            lock = Some(FileLock::acquire(&locks::project_lock_path(path)).map_err(failure)?);
        } else if !matches!(
            self.sessions[&session.project].destination,
            Destination::File(_)
        ) {
            return Err(failure(
                "Library saves use ShellCall::Save; file handoff needs SaveAs",
            ));
        }
        let target = path
            .clone()
            .map_or(SaveTarget::InPlace, |path| SaveTarget::SaveAs {
                path,
                replace,
            });
        let options = self.save_options(&session.project);
        let p = self.project_mut(&session.project).unwrap();
        let workspace = p.workspace().clone();
        p.store_mut().set_workspace(Some(workspace));
        let plan = p
            .store_mut()
            .prepare_save(target, options)
            .map_err(failure)?;
        // Durable before the worker can replace the file the journal is
        // based on; edits while it runs are journalled after this point.
        self.mark_save_point(&session.project, &plan.document, plan.revision);
        let ticket = crate::new_id("save");
        self.sessions
            .get_mut(&session.project)
            .unwrap()
            .pending_save = Some(PendingSave {
            ticket: ticket.clone(),
            path,
            lock,
            revision: plan.revision,
        });
        Ok(SessionSave {
            session,
            ticket,
            plan,
        })
    }
    pub fn finish_session_save(
        &mut self,
        completion: SessionSaveCompletion,
    ) -> Result<ShellReply, KernelFailure> {
        let SessionSaveCompletion {
            session,
            ticket,
            result,
        } = completion;
        self.check_session(&session)?;
        if self.sessions[&session.project]
            .pending_save
            .as_ref()
            .is_none_or(|p| p.ticket != ticket)
        {
            return Err(failure("Save ticket expired"));
        }
        let pending = self
            .sessions
            .get_mut(&session.project)
            .unwrap()
            .pending_save
            .take()
            .unwrap();
        let receipt = result.map_err(failure)?;
        // Written from the snapshot frozen before the error; the slot and the
        // journal base keep their state (recovery offers a copy if they differ).
        self.refuse_faulted(&session.project)?;
        self.project_mut(&session.project)
            .unwrap()
            .store_mut()
            .finish_save(receipt)
            .map_err(failure)?;
        if let Some(path) = pending.path {
            let s = self.sessions.get_mut(&session.project).unwrap();
            s.destination = Destination::File(path);
            s.lock = pending.lock;
        }
        if self.library.is_some() {
            self.save_library_slot(&session.project, &session.project, false)?;
        }
        // Newer edits remain dirty, journalled against the older base. Recovery
        // offers a copy if the file and journal bases differ.
        let store = self.project(&session.project).unwrap().store();
        if store.revision() == pending.revision
            && !store.is_dirty()
            && !store.has_pending_metadata()
        {
            self.rebase_journal(&session.project)?;
        }
        self.flush_project(&session.project)?;
        Ok(ShellReply::Saved {
            session,
            recovery_copy: None,
        })
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
        // A copy that named the original's file would reopen as that file.
        if let Value::Object(m) = &mut meta {
            m.remove("file");
            m.remove("embeddedMedia");
        }
        let ws = p.workspace.unwrap_or(Value::Null);
        let history = p.history.unwrap_or(Value::Null);
        if let Some(file) = file.filter(|f| !f.media.is_empty()) {
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
                    self.save_options(id),
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
    fn compact_all(&mut self, into_files: bool) -> Result<ShellReply, KernelFailure> {
        let handles: Vec<_> = self
            .sessions
            .values()
            .filter(|s| into_files || matches!(s.destination, Destination::Library(_)))
            .filter(|s| self.fault(&s.handle.project).is_none())
            .map(|s| s.handle.clone())
            .collect();
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
            let file = self.file_entry(&id);
            if !into_files && file.is_some() {
                continue;
            }
            // A file's entry has no body: the file is the project.
            let file = file.filter(|path| path.is_file());
            let p = self
                .library
                .as_ref()
                .ok_or_else(|| failure("Attach a native library before compaction"))?
                .store()
                .load(&id)
                .map_err(failure)?;
            let source = match (p, file) {
                (_, Some(path)) => SessionSource::File(path),
                (Some(p), None) => match p
                    .session
                    .as_ref()
                    .and_then(|s| s.get("file"))
                    .and_then(|f| f.get("path"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                {
                    Some(path) if path.is_file() => SessionSource::File(path),
                    _ => SessionSource::Library {
                        profile: self.library.as_ref().unwrap().store().dir().to_path_buf(),
                        project: id.clone(),
                    },
                },
                (None, None) => {
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
                }
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
            if let Destination::Library(slot) = &self.sessions[&session.project].destination
                && slot != &session.project
            {
                recovery_copies.push(slot.clone());
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
        self.open_session_mode(id, source, electron_profile, RecoveryMode::Legacy)
    }
    fn open_session_mode(
        &mut self,
        id: String,
        source: SessionSource,
        electron_profile: Option<PathBuf>,
        mode: RecoveryMode,
    ) -> Result<ShellReply, KernelFailure> {
        if self.project(&id).is_some() {
            return Err(KernelFailure::AlreadyOpen(id));
        }
        if let Some(d) = self.durability() {
            d.barrier()
                .map_err(|e| KernelFailure::Journal(e.to_string()))?;
        }
        // A library entry that stands for an external file is that file. The
        // file route gives it the same journal base, lock and recovery as
        // opening the file itself, so recents never turns a recover into a copy.
        let source = match source {
            SessionSource::Library { profile, project }
                if project == id && self.owns_library(&profile) =>
            {
                match self.file_entry(&project).filter(|p| p.is_file()) {
                    Some(path) => SessionSource::File(path),
                    None => SessionSource::Library { profile, project },
                }
            }
            source => source,
        };
        let original_source = source.clone();
        let mut decision = OpenDecision::Writable;
        let mut lock = None;
        let mut meta = Value::Object(Map::new());
        let mut rejected_history = None;
        let (source, destination, base) = match source {
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
                    OpenSource::Untitled(value),
                    Destination::Library(id.clone()),
                    hash,
                )
            }
            SessionSource::Library { profile, project } => {
                if let Some(s) = self.sessions.values().find(
                    |s| matches!(&s.destination, Destination::Library(slot) if slot == &project),
                ) {
                    return Err(KernelFailure::AlreadyOpen(s.handle.project.clone()));
                }
                let reader = LibraryStore::open(&profile);
                let Some(p) = reader.load(&project).map_err(failure)? else {
                    // A file's entry has no body of its own (save_file_entry):
                    // its unsaved work is in the journal.
                    let file = reader
                        .read(&format!("projectMeta.{project}"))
                        .ok()
                        .flatten()
                        .is_some_and(|m| m.get("file").is_some());
                    if !file {
                        return Err(failure("Library project is missing"));
                    }
                    if !self.owns_library(&profile) {
                        return Err(failure("The project’s file isn’t available"));
                    }
                    return self.open_without_file(id, &project);
                };
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
                let media_only = meta.get("file").is_none() && meta.get("embeddedMedia").is_some();
                let association = meta.get("file").or_else(|| meta.get("embeddedMedia"));
                let associated = association
                    .and_then(|m| m.get("path"))
                    .and_then(Value::as_str)
                    .and_then(|path| {
                        project_io::open_project(std::path::Path::new(path), OpenOptions::default())
                            .ok()
                    })
                    .filter(|opened| {
                        !media_only
                            || association
                                .and_then(|m| m.get("savedHash"))
                                .and_then(Value::as_str)
                                .is_none_or(|hash| {
                                    opened.file.version.as_ref().is_some_and(|v| v.hash == hash)
                                })
                    })
                    .map(|o| o.file);
                let mut target = Destination::Library(project.clone());
                if owned
                    && !media_only
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
                (source, target, hash)
            }

            SessionSource::File(path) => {
                if let Some(s) = self.sessions.values().find(|s| matches!(&s.destination, Destination::File(p) if p.canonicalize().ok().is_some() && p.canonicalize().ok() == path.canonicalize().ok())) { return Err(KernelFailure::AlreadyOpen(s.handle.project.clone())); }
                if let Some(w) = &self.library {
                    // Only the entry's metadata: its project body is the file.
                    let existing = w
                        .store()
                        .read(&format!("projectMeta.{id}"))
                        .map_err(failure)?;
                    let associated = existing
                        .as_ref()
                        .and_then(|m| m.get("file"))
                        .and_then(|m| m.get("path"))
                        .and_then(Value::as_str);
                    if associated.is_some_and(|a| Some(a) == path.to_str()) {
                        // Where the person left off in it.
                        meta = existing.unwrap_or(meta);
                    } else if associated.is_some()
                        || w.store().dir().join(format!("project.{id}.json")).exists()
                    {
                        return Err(failure(
                            "Project id belongs to another library project; open with a fresh session id",
                        ));
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
                let mut opened =
                    project_io::open_project(&path, OpenOptions::default()).map_err(failure)?;
                // The workspace the person left this file with (its entry keeps
                // it, as v1 kept it in the project's state) wins while the file
                // is still the one they left; a file changed elsewhere brings its own.
                let left = meta
                    .get("file")
                    .and_then(|f| f.get("savedHash"))
                    .and_then(Value::as_str);
                if let Some(ws) = meta.get("workspace").filter(|w| w.as_object().is_some())
                    && left.is_some()
                    && left == opened.file.version.as_ref().map(|v| v.hash.as_str())
                {
                    opened.file.envelope.workspace = Some(ws.clone());
                }
                // The opened tree is canonical already: hash it in place
                // (memoized on the document, which recovery reuses).
                let hash = opened
                    .document
                    .canonical_hash()
                    .unwrap_or_else(|| crate::kernel::base_hash_of(&Value::Null));
                (OpenSource::File(opened), Destination::File(path), hash)
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
        // Only a journal left behind needs the base as a value.
        let journaled = self
            .project_dir(recovery_id)
            .is_some_and(|dir| !crate::journal::sessions(&dir).is_empty());
        let mut recovered = Recovery::Nothing;
        if journaled {
            let document = source_document(&source);
            let saved = saved_state(&source);
            recovered = self.recover_from(
                recovery_id,
                Some(&base),
                Some(crate::journal::Base {
                    document: &document,
                    history: saved.as_ref().map(|(h, _)| h),
                    workspace: saved.as_ref().map(|(_, w)| w),
                }),
            );
            // A journal of an older base can also end where the file is now
            // (a save that could not start a new journal before the kill).
            if matches!(mode, RecoveryMode::Ask)
                && let Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } = &recovered
                && !r.truncated
                && r.document.canonical_hash().as_deref() == Some(&base)
                && let Some((history, workspace)) = &saved
                && (r.history.saves_as(history) || r.history.regroups(history))
                && r.workspace == *workspace
            {
                // The base already holds this document, its layout and every
                // undo state, so there is nothing to decide. A coarser
                // grouping of the saved steps (an agent's finish_session
                // squash after its save) is not kept: the saved steps reach
                // every state it did. Old sessions stay on disk.
                recovered = Recovery::Nothing;
            }
        }
        let file_stamp = recovery_file
            .as_ref()
            .and_then(|f| f.version.as_ref())
            .map(|v| v.hash.as_str())
            .unwrap_or("");
        let stamp = format!(
            "{file_stamp}:{}",
            match &recovered {
                Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } => format!(
                    "{}:{}:{}:{}",
                    base,
                    r.session.display(),
                    r.revision,
                    r.document.canonical_hash().unwrap_or_default()
                ),
                Recovery::Unrecoverable { session, reason } =>
                    format!("{base}:{session:?}:{reason}"),
                Recovery::Nothing => format!("{base}:none"),
            }
        );
        if matches!(mode, RecoveryMode::Ask) && !matches!(recovered, Recovery::Nothing) {
            let recovery = match &recovered {
                Recovery::Recovered(r) => RecoveryResult::Recovered {
                    truncated: r.truncated,
                },
                Recovery::Copy { recovered: r, .. } => RecoveryResult::OfferCopy {
                    project: id.clone(),
                    truncated: r.truncated,
                },
                Recovery::Unrecoverable { reason, .. } => RecoveryResult::Unrecoverable {
                    reason: reason.clone(),
                },
                _ => unreachable!(),
            };
            self.pending_recovery.retain(|_, p| p.id != id);
            let ticket = crate::new_id("recovery-choice");
            let journals = self.project_dir(recovery_id);
            let library = self.library.as_ref().map(|w| w.store().dir().to_path_buf());
            let prepared = Prepared {
                id: id.clone(),
                source,
                destination,
                lock,
                decision,
                meta,
                rejected_history,
                base,
                recovered,
                recovery_file,
            };
            let fresh = freshness(prepared.inputs(journals, library.as_deref()));
            self.pending_recovery.insert(
                ticket.clone(),
                PendingRecovery {
                    id,
                    source: original_source,
                    electron_profile,
                    stamp,
                    prepared: Some(Box::new(prepared)),
                    fresh,
                },
            );
            return Ok(ShellReply::RecoveryRequired { ticket, recovery });
        }
        let choice = match &mode {
            RecoveryMode::Decide(choice, expected) => {
                if expected != &stamp {
                    return Err(failure("Recovery base changed; prepare the open again"));
                }
                Some(*choice)
            }
            _ => None,
        };
        self.settle_open(
            Prepared {
                id,
                source,
                destination,
                lock,
                decision,
                meta,
                rejected_history,
                base,
                recovered,
                recovery_file,
            },
            choice,
        )
    }
    /// The rest of an open once any recovery decision is made.
    fn settle_open(
        &mut self,
        prepared: Prepared,
        choice: Option<RecoveryDecision>,
    ) -> Result<ShellReply, KernelFailure> {
        let Prepared {
            id,
            mut source,
            mut destination,
            mut lock,
            decision,
            meta,
            mut rejected_history,
            base,
            recovered,
            recovery_file,
        } = prepared;
        let setting_aside = matches!(
            choice,
            Some(RecoveryDecision::Discard | RecoveryDecision::OpenSavedVersion)
        );
        let unrecoverable = matches!(&recovered, Recovery::Unrecoverable { .. });
        if (choice.is_some() && !setting_aside && unrecoverable)
            || (choice == Some(RecoveryDecision::OpenSavedVersion) && !unrecoverable)
        {
            return Err(failure(
                "Recovery could not be replayed; files are retained",
            ));
        }
        if setting_aside {
            // Explicitly rejected sessions remain archived. No save of recovered
            // content, and no unrelated journal session is retired.
            let paths = match &recovered {
                Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } => {
                    vec![r.session.clone()]
                }
                Recovery::Unrecoverable { session, .. } => vec![session.clone()],
                _ => vec![],
            };
            for path in paths {
                crate::journal::archive_session(path).map_err(failure)?;
            }
        }
        if matches!(
            choice,
            Some(RecoveryDecision::OpenAsCopy | RecoveryDecision::Recover)
        ) && (choice == Some(RecoveryDecision::OpenAsCopy)
            || matches!(&recovered, Recovery::Copy { .. }))
        {
            let r = match recovered {
                Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } => r,
                _ => {
                    return Err(failure(
                        "Recovery could not be replayed; files are retained",
                    ));
                }
            };
            let crashed = r.session.clone();
            let copy = self.recovery_copy(r, recovery_file)?;
            // The copy holds that work now; left live, every later open and
            // quit would replay it again.
            crate::journal::archive_session(crashed).map_err(failure)?;
            let profile = self.library.as_ref().unwrap().store().dir().to_path_buf();
            return self.open_session(
                id,
                SessionSource::Library {
                    profile,
                    project: copy,
                },
                None,
            );
        }
        let recovered = if setting_aside {
            Recovery::Nothing
        } else {
            recovered
        };
        let recovery = match recovered {
            Recovery::Nothing => RecoveryResult::None,
            Recovery::Unrecoverable { reason, .. } => RecoveryResult::Unrecoverable { reason },
            Recovery::Copy { recovered, .. } => {
                let truncated = recovered.truncated;
                let crashed = recovered.session.clone();
                let copy = self.recovery_copy(recovered, recovery_file.clone())?;
                crate::journal::archive_session(crashed).map_err(failure)?;
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
        // Unless recovery replaced it, the document opened is the one hashed.
        let known = matches!(
            source,
            OpenSource::File(_)
                | OpenSource::Library(_)
                | OpenSource::LibraryWithFile { .. }
                | OpenSource::Untitled(_)
        )
        .then(|| base.clone());
        // The library slot already holds exactly what opened from it.
        let slot_current = known.is_some()
            && match (&source, &destination) {
                (
                    OpenSource::Library(lib) | OpenSource::LibraryWithFile { lib, .. },
                    Destination::Library(slot),
                ) => slot == &lib.id && lib.journal_applied == 0 && !lib.from_autosave,
                _ => false,
            };
        let mut info = self.open_known(&id, source, known)?;
        let slot_current = slot_current
            && !info.history_rejected
            && !self.project(&id).unwrap().store().upgraded_on_open();
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
                pending_save: None,
            },
        );
        // A new/copy project gets a rollback-readable slot immediately.
        if slot_current {
            // Reopening a library project rewrites none of it, however large.
            let p = self.project(&handle.project).unwrap();
            let summary = entry_summary(p.store().root());
            let session = &self.sessions[&handle.project];
            let Destination::Library(slot) = &session.destination else {
                unreachable!("slot_current is a library destination")
            };
            if let Err(e) = self
                .library
                .as_ref()
                .ok_or_else(|| failure("Attach a native library first"))
                .and_then(|w| {
                    w.touch_entry(slot, &summary, session.meta.clone())
                        .map_err(failure)
                })
            {
                self.sessions.remove(&handle.project);
                self.close(&handle.project);
                return Err(e);
            }
        } else if matches!(
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
        } else if self.library.is_some()
            && let Err(e) = self.save_library_slot(&handle.project, &handle.project, false)
        {
            self.sessions.remove(&handle.project);
            self.close(&handle.project);
            return Err(e);
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
        if let Some(file) = file.filter(|f| !f.media.is_empty()) {
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
                    self.save_options(&id),
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
        save_as: Option<(PathBuf, bool)>,
    ) -> Result<ShellReply, KernelFailure> {
        self.ensure_idle(handle)?;
        let id = &handle.project;
        let mut new_lock = None;
        if let Some((path, _)) = &save_as {
            let decision = locks::open_decision(path, None);
            if decision != OpenDecision::Writable {
                return Err(failure(format!(
                    "Choose a writable local destination: {decision:?}"
                )));
            }
            new_lock = Some(FileLock::acquire(&locks::project_lock_path(path)).map_err(failure)?);
        }
        let mut recovery_copy = None;
        let file_path = save_as.as_ref().map(|(path, _)| path.clone()).or_else(|| {
            match &self.sessions[id].destination {
                Destination::File(path) => Some(path.clone()),
                _ => None,
            }
        });
        if let Some(path) = file_path {
            let ws = self.project(id).unwrap().workspace().clone();
            let options = self.save_options(id);
            let p = self.project_mut(id).unwrap();
            p.store_mut().set_workspace(Some(ws));
            let target = match &save_as {
                Some((path, replace)) => SaveTarget::SaveAs {
                    path: path.clone(),
                    replace: *replace,
                },
                None => SaveTarget::InPlace,
            };
            let plan = p.store_mut().prepare_save(target, options);
            if let Ok(plan) = &plan {
                self.mark_save_point(id, &plan.document, plan.revision);
            }
            let p = self.project_mut(id).unwrap();
            let result = plan
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
        if !recovery && slot == id && matches!(self.sessions[id].destination, Destination::File(_))
        {
            return self.save_file_entry(id);
        }
        let mut value = realize(self, id)?;
        let name = if recovery {
            Some(format!(
                "{} — Recovered",
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Project")
            ))
        } else if value.get("id").and_then(Value::as_str) != Some(slot)
            && matches!(self.sessions[id].destination, Destination::Library(_))
        {
            // A copy is told apart from its source. A file's own entry is the
            // file, under its own name (plan §6: external files open in place).
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
        let store_dir = self
            .library
            .as_ref()
            .and_then(|w| w.store().dir().parent())
            .map(|profile| profile.join(crate::library_media::DIR));
        // Bytes the library's media store keeps need no backing file.
        if self.project(id).unwrap().store().media().iter().any(|e| {
            !store_dir
                .as_deref()
                .is_some_and(|d| crate::library_media::is_stored(e, d))
        }) && (recovery || matches!(self.sessions[id].destination, Destination::Library(_)))
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
            let options = self.save_options(slot);
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
                .prepare_save(target, options)
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
            .filter(|_| p.store().history().is_empty())
            .unwrap_or_else(|| p.store().history().snapshot().to_saved_packed());
        let mut meta = self
            .state
            .ui
            .get(id)
            .map(|ui| ui.to_session())
            .unwrap_or_else(|| session.meta.clone());
        // Only the file's own entry, or a copy's backing file in the profile,
        // names a file. A copy that named the original would reopen as it.
        let backing = self
            .library
            .as_ref()
            .and_then(|w| w.store().dir().parent())
            .map(|profile| profile.join("recovery-files"));
        let own_file = match &session.destination {
            Destination::File(_) => true,
            Destination::Library(_) => p
                .store()
                .path()
                .zip(backing.as_deref())
                .is_some_and(|(path, backing)| path.starts_with(backing)),
        };
        if own_file && p.store().path().is_some() {
            meta = file_metadata(meta, p.store());
        } else if let Value::Object(m) = &mut meta {
            m.remove("file");
        }
        let writer = self
            .library
            .as_ref()
            .ok_or_else(|| failure("Attach a native library first"))?;
        let result = if self.compacting {
            writer.compact_project(slot, value, history, p.workspace().clone(), meta.clone())
        } else {
            writer.save_project(slot, value, history, p.workspace().clone(), meta.clone())
        };
        result.map_err(failure)?;
        self.sessions.get_mut(id).unwrap().meta = meta;
        // A library snapshot is crash recovery for a file session, not proof
        // that the external file contains newer edits made during a save.
        if recovery || matches!(self.sessions[id].destination, Destination::Library(_)) {
            self.project_mut(id)
                .unwrap()
                .store_mut()
                .mark_library_saved();
        }
        Ok(())
    }
}

impl Kernel {
    fn owns_library(&self, profile: &std::path::Path) -> bool {
        self.library.as_ref().is_some_and(|w| {
            w.store().dir().canonicalize().ok()
                == LibraryStore::open(profile).dir().canonicalize().ok()
        })
    }
    /// The external file a library entry stands for (plan §6: a file opened
    /// from disk is its own entry). A recovery copy's backing file is not one.
    fn file_entry(&self, id: &str) -> Option<PathBuf> {
        let w = self.library.as_ref()?;
        let meta = w.store().read(&format!("projectMeta.{id}")).ok()??;
        let path = PathBuf::from(meta.get("file")?.get("path")?.as_str()?);
        let backing = w.store().dir().parent()?.join("recovery-files");
        (!path.starts_with(backing)).then_some(path)
    }
    /// A closed file entry as a library project, read from its file.
    /// A file entry whose file is gone: its journal's work opens as a
    /// recovered copy in the library; the entry stays, naming the file.
    fn open_without_file(
        &mut self,
        id: String,
        project: &str,
    ) -> Result<ShellReply, KernelFailure> {
        match self.recover(project, None, None) {
            Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } => {
                let crashed = r.session.clone();
                let copy = self.recovery_copy(r, None)?;
                crate::journal::archive_session(crashed).map_err(failure)?;
                let profile = self.library.as_ref().unwrap().store().dir().to_path_buf();
                self.open_session(
                    id,
                    SessionSource::Library {
                        profile,
                        project: copy,
                    },
                    None,
                )
            }
            Recovery::Unrecoverable { .. } => Err(failure(
                "The project’s file isn’t available, and its unsaved changes can’t be replayed without it. They are kept.",
            )),
            Recovery::Nothing => Err(failure("The project’s file isn’t available")),
        }
    }
    fn file_entry_project(
        &self,
        id: &str,
    ) -> Result<crate::library::LibraryProject, KernelFailure> {
        let Some(path) = self.file_entry(id).filter(|p| p.is_file()) else {
            // The file is gone: its journal holds the newest state, if any.
            return match self.recover(id, None, None) {
                Recovery::Recovered(r) | Recovery::Copy { recovered: r, .. } => {
                    Ok(crate::library::LibraryProject {
                        id: id.into(),
                        project: pm_model::canonical::realize_project(
                            r.document.to_value().unwrap_or(Value::Null),
                        ),
                        history: Some(r.history.snapshot().to_saved_packed()),
                        workspace: Some(r.workspace),
                        session: self.library.as_ref().and_then(|w| {
                            w.store().read(&format!("projectMeta.{id}")).ok().flatten()
                        }),
                        journal_applied: 0,
                        journal_skipped: 0,
                        from_autosave: false,
                    })
                }
                _ => Err(failure("The project’s file isn’t available")),
            };
        };
        let opened = project_io::open_project(&path, OpenOptions::default()).map_err(failure)?;
        let session = self
            .library
            .as_ref()
            .and_then(|w| w.store().read(&format!("projectMeta.{id}")).ok().flatten());
        Ok(crate::library::LibraryProject {
            id: id.into(),
            project: pm_model::canonical::realize_project(
                opened.document.to_value().unwrap_or(Value::Null),
            ),
            history: opened.history.map(|h| h.snapshot().to_saved_packed()),
            workspace: opened.file.envelope.workspace.clone(),
            session,
            journal_applied: 0,
            journal_skipped: 0,
            from_autosave: false,
        })
    }
    /// A file session's library entry: its row and where the file is. The
    /// file is the project; the journal is its crash recovery.
    fn save_file_entry(&mut self, id: &str) -> Result<(), KernelFailure> {
        let p = self.project(id).unwrap();
        let session = &self.sessions[id];
        let meta = self
            .state
            .ui
            .get(id)
            .map(|ui| ui.to_session())
            .unwrap_or_else(|| session.meta.clone());
        let meta = with_workspace(file_metadata(meta, p.store()), p.workspace());
        let summary = entry_summary(p.store().root());
        let writer = self
            .library
            .as_ref()
            .ok_or_else(|| failure("Attach a native library first"))?;
        writer
            .save_file_entry(id, &summary, meta.clone(), self.compacting)
            .map_err(failure)?;
        self.sessions.get_mut(id).unwrap().meta = meta;
        Ok(())
    }
    /// The library entry's view state alone, for a file closed unchanged:
    /// where the person left off, without rewriting the project body.
    fn save_session_meta(&mut self, id: &str) -> Result<(), KernelFailure> {
        let Some(writer) = &self.library else {
            return Ok(());
        };
        let p = self.project(id).unwrap();
        let session = &self.sessions[id];
        let meta = self
            .state
            .ui
            .get(id)
            .map(|ui| ui.to_session())
            .unwrap_or_else(|| session.meta.clone());
        let meta = with_workspace(file_metadata(meta, p.store()), p.workspace());
        writer
            .set(&format!("projectMeta.{id}"), meta.clone())
            .and_then(|()| writer.flush())
            .map_err(failure)?;
        self.sessions.get_mut(id).unwrap().meta = meta;
        Ok(())
    }
}

/// A library row's `name`, `w` and `h`, as the realized project has them
/// (the open composition's size), read without realizing the document.
fn entry_summary(root: Option<&pm_model::doc::Node>) -> Value {
    let mut m = Map::new();
    let Some(root) = root else {
        return Value::Object(m);
    };
    let comp = root
        .get("compId")
        .and_then(|id| match id.kind() {
            pm_model::doc::NodeKind::String(s) => Some(s.clone()),
            _ => None,
        })
        .and_then(|id| root.get("comps")?.get(id.as_str()?).cloned());
    if let Some(name) = root.get("name") {
        m.insert("name", name.to_value());
    }
    for k in ["w", "h"] {
        if let Some(v) = comp.as_ref().and_then(|c| c.get(k)).or_else(|| root.get(k)) {
            m.insert(k, v.to_value());
        }
    }
    Value::Object(m)
}

/// A file entry's metadata with the project's workspace, so reopening the
/// file restores the panels the person left (v1's project state did).
fn with_workspace(meta: Value, workspace: &Value) -> Value {
    let mut m = meta.as_object().cloned().unwrap_or_default();
    if workspace.as_object().is_some() {
        m.insert("workspace", workspace.clone());
    }
    Value::Object(m)
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
