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
    /// Library copies holding work for the file this session has open in
    /// place that the person has not been told about yet (see
    /// `Kernel::unoffered_copies`). Replies `Copies`.
    CopiesOf(SessionHandle),
    /// Files kept beside this session's project file that the person has
    /// not seen a notice about yet: a version that arrived during a save,
    /// what a save that was cut short left, a retained version found
    /// changed. They are read from the saves' own records beside the file
    /// (written before each file was kept, so a crash loses none). A record
    /// that does not know the file now open (it was changed since by
    /// another program) is told as `earlier`. Replies `Kept`; nothing is
    /// forgotten until `KeptSeen`.
    KeptBeside(SessionHandle),
    /// The person was shown, and answered, the notices about the kept
    /// files `paths` (and, with `restored`, the ones about no file: that
    /// the last saved version was put back). Exactly those are not reported
    /// again; with `other`, the session's own rows about them go too.
    KeptSeen {
        session: SessionHandle,
        paths: Vec<PathBuf>,
        restored: bool,
        other: bool,
    },
    /// The files a save of the project file at this path left when it was
    /// cut short and recovery would not choose between (their bytes are
    /// not what the save's record says, or more than one save was cut
    /// short). Moves nothing. Replies `Leftovers`.
    Leftovers(PathBuf),
    /// The person chose `leftover` (a path `Leftovers` listed) to be the
    /// project file at `path`, whose name holds nothing: it takes that name
    /// (never over a file that landed there since), and the other files
    /// that save left get names the person finds. Replies `Kept` with them.
    UseLeftover {
        path: PathBuf,
        leftover: PathBuf,
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
    /// The last call of a quit: flush and unlock the library. A session
    /// still open is closed as `DeferClose` closes it. Later library calls
    /// fail.
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
    /// `recovered` is another file entry's work (its id), offered to this
    /// file: see [`RecoveryResult::FromOtherFile`].
    from_other: Option<String>,
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
    Recovered {
        truncated: bool,
    },
    OfferCopy {
        project: String,
        truncated: bool,
    },
    Unrecoverable {
        reason: String,
    },
    /// Unsaved work left for another file (`name`, no longer where it was)
    /// whose saved content is exactly this file's. It may be this file
    /// (moved across volumes, restored from a backup) or a duplicate of it:
    /// the person decides. Until they choose Recover nothing of that work
    /// is touched, and `Discard` leaves it where it is.
    FromOtherFile {
        name: String,
        truncated: bool,
    },
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
        /// Another version of the file arrived while it was being saved:
        /// the save is published and that version is kept beside it here.
        displaced: Option<PathBuf>,
    },
    Closed {
        recovery_copy: Option<String>,
    },
    Durable,
    Recent(Vec<Value>),
    /// Library rows (`id`, `name`, `at`) of copies of a file.
    Copies(Vec<Value>),
    /// Rows (`why`, and `path` unless `why` is `restored`) of files kept
    /// beside a project file. `why`: `arrived` (another version, taken from
    /// the path by a save that stands), `restored` (a save was cut short and
    /// the last saved version is back under the project's name), `other`
    /// (the file such a save had moved aside, where the path holds a file),
    /// `unfinished` (the finished file of the save that was cut short),
    /// `undecided` (a file such a save left that recovery would not move),
    /// `unconfirmed` (the copy kept on this machine of the last save to a
    /// volume that could not prove it durable, when the file read is not
    /// that save).
    Kept(Vec<Value>),
    /// Rows of files a cut-short save left that the person has to choose
    /// between: `path`, `role` (`previous`: the file that was under the
    /// project's name; `saving`: the file the save had finished; `record`:
    /// a save's record that cannot be read), `why` (`unknown`, `several`,
    /// `unreadable`; `active`: the save is going on right now in another
    /// copy of the app on this Mac, and nothing can be chosen; `elsewhere`:
    /// it was begun on another computer on a shared volume, which may have
    /// crashed or may still be at it), `size`, and who was saving and when
    /// (`computer`, `user`, `at` in milliseconds) where the record says.
    Leftovers(Vec<Value>),
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
    /// A save whose file was installed but whose folder did not sync after
    /// it ([`crate::ProjectIoError::Finalize`]): adopted by the next save in
    /// place once the folder syncs, so that save expects the file that is
    /// there.
    pub unfinalized: Option<Box<crate::SaveReceipt>>,
}
/// The session metadata member that lists files kept beside the project
/// file which the person has not been told about.
const KEPT_BESIDE: &str = "keptBeside";
fn kept_row(why: &str, path: Option<&std::path::Path>) -> Value {
    let mut m = Map::new();
    m.insert("why", Value::from(why));
    if let Some(path) = path {
        m.insert("path", Value::from(path.to_string_lossy().into_owned()));
    }
    Value::Object(m)
}
fn left_row(left: &project_io::LeftAside) -> Value {
    use project_io::LeftAside::*;
    match left {
        Restored => kept_row("restored", None),
        Kept(path) => kept_row("other", Some(path)),
        Unfinished(path) => kept_row("unfinished", Some(path)),
    }
}
fn leftover_row(left: &project_io::Leftover) -> Value {
    let mut m = Map::new();
    m.insert(
        "path",
        Value::from(left.path.to_string_lossy().into_owned()),
    );
    m.insert("role", Value::from(left.role));
    m.insert(
        "why",
        Value::from(match left.why {
            project_io::Undecided::Unknown => "unknown",
            project_io::Undecided::Several => "several",
            project_io::Undecided::Unreadable => "unreadable",
            project_io::Undecided::Active => "active",
            project_io::Undecided::Elsewhere => "elsewhere",
            project_io::Undecided::Proven => "proven",
        }),
    );
    m.insert("size", Value::from(left.size as f64));
    if let Some(owner) = &left.owner {
        m.insert("computer", Value::from(owner.computer.as_str()));
        m.insert("user", Value::from(owner.user.as_str()));
        m.insert("at", Value::from(left.at as f64));
    }
    Value::Object(m)
}
fn push_kept(meta: &mut Map, rows: impl IntoIterator<Item = Value>) {
    let mut rows = rows.into_iter().peekable();
    if rows.peek().is_none() {
        return;
    }
    let mut kept = match meta.remove(KEPT_BESIDE) {
        Some(Value::Array(kept)) => kept,
        _ => Vec::new(),
    };
    kept.extend(rows);
    meta.insert(KEPT_BESIDE, Value::Array(kept));
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
        let reply = self.shell_call(call);
        self.tell_strays();
        reply
    }
    fn shell_call(&mut self, call: ShellCall) -> Result<ShellReply, KernelFailure> {
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
            ShellCall::CopiesOf(handle) => {
                self.check_session(&handle)?;
                Ok(ShellReply::Copies(self.unoffered_copies(&handle.project)))
            }
            ShellCall::KeptBeside(handle) => {
                self.check_session(&handle)?;
                let mut rows = Vec::new();
                if let Some(Destination::File(path)) =
                    self.sessions.get(&handle.project).map(|s| &s.destination)
                {
                    let path = path.clone();
                    let hash = self
                        .project(&handle.project)
                        .and_then(|p| p.store().file().version.clone())
                        .map(|v| v.hash.clone());
                    rows.extend(
                        project_io::kept_notices(&path, hash.as_deref())
                            .iter()
                            .map(|(why, kept)| kept_row(why, kept.as_deref())),
                    );
                }
                if let Some(Value::Array(more)) = self
                    .state
                    .ui
                    .get(&handle.project)
                    .and_then(|ui| ui.session.get(KEPT_BESIDE))
                {
                    rows.extend(more.iter().cloned());
                }
                Ok(ShellReply::Kept(rows))
            }
            ShellCall::KeptSeen {
                session,
                paths,
                restored,
                other,
            } => {
                self.check_session(&session)?;
                if let Some(Destination::File(path)) =
                    self.sessions.get(&session.project).map(|s| &s.destination)
                {
                    project_io::kept_seen(path, &paths, restored);
                }
                // The session's own rows: exactly the ones that were shown.
                let shown = |row: &Value| match row.get("path").and_then(Value::as_str) {
                    Some(path) => paths.iter().any(|p| p.as_os_str() == path),
                    None => restored,
                };
                let rows = std::sync::Arc::make_mut(&mut self.state.ui)
                    .get_mut(&session.project)
                    .and_then(|ui| ui.session.as_object_mut());
                if other
                    && let Some(meta) = rows
                    && let Some(Value::Array(mut kept)) = meta.remove(KEPT_BESIDE)
                {
                    let before = kept.len();
                    kept.retain(|row| !shown(row));
                    let changed = kept.len() != before;
                    if !kept.is_empty() {
                        meta.insert(KEPT_BESIDE, Value::Array(kept));
                    }
                    if changed {
                        self.save_session_meta(&session.project)?;
                    }
                }
                Ok(ShellReply::Kept(Vec::new()))
            }
            ShellCall::Leftovers(path) => Ok(ShellReply::Leftovers(
                project_io::leftovers(&path)
                    .iter()
                    .map(leftover_row)
                    .collect(),
            )),
            ShellCall::UseLeftover { path, leftover } => {
                // Under the project's lock: no save of it is in its steps.
                let _lock = self.lock_project(&path).map_err(failure)?;
                let left =
                    project_io::use_leftover(&path, &leftover, &self.host()).map_err(failure)?;
                Ok(ShellReply::Kept(left.iter().map(left_row).collect()))
            }
            ShellCall::DecideRecovery { ticket, decision } => {
                let mut pending = self
                    .pending_recovery
                    .remove(&ticket)
                    .ok_or_else(|| failure("Recovery choice expired; prepare the open again"))?;
                if let Some(prepared) = pending.prepared.take() {
                    let journals =
                        self.project_dir(match (&prepared.from_other, &prepared.destination) {
                            (Some(other), _) => other,
                            (None, Destination::Library(slot)) => slot,
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
                // A quit closed every project a window shows before this. One
                // still open has no window to ask in (an agent's project
                // nobody showed, an open nothing adopted): it leaves as a
                // deferred close leaves it, its acknowledged edits durable in
                // its journal for the next open to offer, its file as saved.
                // Refusing here refused every later quit as well.
                let unshown: Vec<String> = self.sessions.keys().cloned().collect();
                for project in unshown {
                    self.flush_project(&project)?;
                    self.remove_closed_project(&project);
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
                    let ShellReply::Saved {
                        recovery_copy,
                        displaced,
                        ..
                    } = saved
                    else {
                        unreachable!()
                    };
                    // Nobody sees this save's reply: the save's record keeps
                    // what it kept, and the next open says it.
                    let _ = displaced;
                    recovery_copy
                };
                crate::fault_injection::hit(crate::fault_injection::Stage::InClose);
                self.flush_project(&handle.project)?;
                self.durability_barrier()?;
                let journals = self.project_dir(&handle.project);
                // A volume that could not prove the save durable may still
                // lose it: the journal stays (and its local copy, see `local_copy`).
                let proven = !self.unproven.contains(&handle.project);
                self.remove_closed_project(&handle.project);
                // Saved where it belongs and durable: the journal holds nothing
                // more. Left behind, every quit reopened the project to compact
                // it, and a later open asked to recover work that was saved.
                if recovery_copy.is_none()
                    && proven
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
        let prepared = self.prepare_save_of(session, path, replace);
        self.tell_strays();
        prepared
    }
    fn prepare_save_of(
        &mut self,
        session: SessionHandle,
        path: Option<PathBuf>,
        replace: bool,
    ) -> Result<SessionSave, KernelFailure> {
        self.ensure_idle(&session)?;
        if path.is_none() {
            self.adopt_unfinalized(&session.project)?;
            self.follow_file(&session.project);
        }
        let mut lock = None;
        if let Some(path) = &path {
            if locks::open_decision(path, None) != OpenDecision::Writable {
                return Err(failure("Choose a writable local destination"));
            }
            lock = Some(self.lock_project(path).map_err(failure)?);
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
        // Without it the save does not start (the plan and lock drop here).
        self.mark_save_point(&session.project, &plan.document, plan.revision)?;
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
        let finished = self.finish_save_of(completion);
        self.tell_strays();
        finished
    }
    fn finish_save_of(
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
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(crate::ProjectIoError::Finalize {
                message,
                receipt: Some(mut receipt),
            }) => {
                // Kept as a ticket, not as a save still running.
                receipt.ticket = std::sync::Arc::new(());
                self.sessions.get_mut(&session.project).unwrap().unfinalized = Some(receipt);
                return Err(failure(crate::ProjectIoError::Finalize {
                    message,
                    receipt: None,
                }));
            }
            Err(e) => {
                self.adopt_put_back(&session.project, &e);
                return Err(failure(e));
            }
        };
        self.sessions.get_mut(&session.project).unwrap().unfinalized = None;
        let displaced = self.note_changed(&session.project, &receipt);
        self.note_unproven(&session.project, receipt.unproven);
        // Written from the snapshot frozen before the error; the slot and the
        // journal base keep their state (recovery offers a copy if they differ).
        self.refuse_faulted(&session.project)?;
        self.project_mut(&session.project)
            .unwrap()
            .store_mut()
            .finish_save(receipt)
            .map_err(failure)?;
        self.save_published(&session.project);
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
            displaced,
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
            if let ShellReply::Saved {
                recovery_copy,
                displaced,
                ..
            } = self.shell(ShellCall::Save(handle.clone()))?
            {
                projects += 1;
                recovery_copies.extend(recovery_copy);
                // A quit has no window to say it in: the save's record keeps
                // what it kept, and the next open says it.
                let _ = displaced;
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
            // From here on the project is one no window has open: its
            // journal is folded into its slot if that works. Anything that
            // does not (an unreadable slot or journal, a recovery that needs
            // the person's decision, a close that fails) leaves the journal
            // where it is for the next open to offer. It never fails the
            // quit: a quit refused for a project that is not even open could
            // be refused forever.
            let Ok(p) = self
                .library
                .as_ref()
                .ok_or_else(|| failure("Attach a native library before compaction"))?
                .store()
                .load(&id)
            else {
                continue;
            };
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
                            if let Ok(copy) = self.recovery_copy(r, None) {
                                recovery_copies.push(copy);
                                projects += 1;
                            }
                        }
                        Recovery::Nothing | Recovery::Unrecoverable { .. } => {}
                    }
                    continue;
                }
            };
            // Asked first, with nobody here to answer. Only a journal that
            // replays whole is folded in. One this build cannot replay, or
            // whose end is missing, is the person's to decide at the next
            // open: the project is not opened at all, so nothing saves its
            // older slot and nothing removes the journal as saved.
            let opened =
                match self.open_session_mode(id.clone(), source.clone(), None, RecoveryMode::Ask) {
                    Ok(ShellReply::RecoveryRequired { ticket, recovery }) => {
                        self.pending_recovery.remove(&ticket);
                        if !matches!(
                            recovery,
                            RecoveryResult::Recovered { truncated: false }
                                | RecoveryResult::OfferCopy {
                                    truncated: false,
                                    ..
                                }
                        ) {
                            continue;
                        }
                        self.open_session(id, source, None)
                    }
                    // Nothing to recover: it is open.
                    opened => opened,
                };
            let Ok(ShellReply::Opened {
                session, recovery, ..
            }) = opened
            else {
                continue;
            };
            if let RecoveryResult::OfferCopy { project, .. } = recovery {
                recovery_copies.push(project);
            }
            if let Destination::Library(slot) = &self.sessions[&session.project].destination
                && slot != &session.project
            {
                recovery_copies.push(slot.clone());
            }
            // A close that fails leaves the session to the library's
            // release, which defers it (journal kept).
            if let Ok(ShellReply::Closed { recovery_copy }) = self.shell(ShellCall::Close(session))
            {
                recovery_copies.extend(recovery_copy);
                projects += 1;
            }
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
                // A file a save was cut short on is still that file: its last
                // saved version waits beside its path, and the file route
                // puts it back.
                match self
                    .file_entry(&project)
                    .filter(|p| p.is_file() || project_io::pending(p))
                {
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
        // A file entry whose unsaved work may be this file's (id, file name).
        let mut maybe_left: Option<(String, String)> = None;
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
                    .map(std::path::Path::new)
                    .and_then(|path| {
                        if media_only {
                            // A migrated copy: the original app's file lends
                            // media per asset, never as a whole.
                            let store = owned.then(|| {
                                reader
                                    .dir()
                                    .parent()
                                    .map(|p| p.join(crate::library_media::DIR))
                            });
                            crate::embedded_media::for_copy(path, &p, store.flatten().as_deref())
                        } else {
                            project_io::open_project(path, OpenOptions::default())
                                .ok()
                                .map(|o| o.file)
                        }
                    });
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
                                match self.lock_project(path) {
                                    Ok(l) => {
                                        lock = Some(l);
                                        crate::project_io::remove_abandoned_temps(path);
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
                let mut left_aside = project_io::Settled::default();
                if decision == OpenDecision::Writable {
                    match self.lock_project(&path) {
                        Ok(l) => {
                            lock = Some(l);
                            // Staging files a save cut short by Quit Now left.
                            crate::project_io::remove_abandoned_temps(&path);
                            // What a save that was cut short left is settled
                            // from its record: the version it moved aside
                            // and never got to replace is the file again.
                            left_aside = project_io::settle(&path, &self.host());
                            self.log_unrecorded(&left_aside);
                            if let Some(e) = &left_aside.unsynced {
                                return Err(failure(format!(
                                    "The last save of this project did not finish. Its files \
                                     were put back in order, but the folder could not be \
                                     synced to make that last ({e}). Nothing was removed. \
                                     Try opening it again."
                                )));
                            }
                        }
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
                // The copy made the last time this file could not open in
                // place holds the person's work since: the same file opens
                // that copy again, never a second one. A different file at
                // that name (replaced, or a link pointed elsewhere) is not
                // that copy's file: it gets a copy of its own below.
                if decision != OpenDecision::Writable
                    && let Some(copy) = self.copy_of(&path)
                {
                    return self.reopen_copy(id, copy, decision, mode);
                }
                // Its name holds nothing and its last saved version waits
                // beside it, but the lock is not ours: another computer is
                // between the two renames of a save, or its owner ended there
                // moments ago (a share's lock is believed for 30 seconds).
                // Nothing is moved without the lock.
                if decision != OpenDecision::Writable
                    && !path.is_file()
                    && project_io::pending(&path)
                {
                    return Err(failure(
                        "This project was being saved a moment ago, here or on another computer, \
                         and that save did not finish. Its last saved version is safe next to \
                         it. Try opening it again in a minute.",
                    ));
                }
                // The name holds nothing, and which of the files the save
                // left is the project could not be told from its record:
                // nothing was moved, and the person chooses
                // (`ShellCall::Leftovers`, `ShellCall::UseLeftover`).
                if !left_aside.undecided.is_empty() && !path.is_file() {
                    let names: Vec<_> = left_aside
                        .undecided
                        .iter()
                        .filter_map(|l| Some(l.path.file_name()?.to_string_lossy().into_owned()))
                        .collect();
                    // Somebody else's save: who, and since when.
                    let held = left_aside.undecided.iter().find_map(|l| {
                        let owner = l.owner.as_ref()?;
                        let since = project_io::local_time(l.at / 1000);
                        match l.why {
                            project_io::Undecided::Active => Some(format!(
                                "This project is being saved right now by another copy of                                  Powermove on this Mac (user {}, since {since}). Nothing was                                  moved. Open it again when that save has finished.",
                                owner.user
                            )),
                            project_io::Undecided::Elsewhere => Some(format!(
                                "A save of this project from the computer “{}” (user {}),                                  begun {since}, has not finished: that computer may still be                                  saving, or it stopped part way. Nothing was moved or removed.                                  Its files are next to where the project was: {}",
                                owner.computer,
                                owner.user,
                                names.join(", ")
                            )),
                            _ => None,
                        }
                    });
                    if let Some(held) = held {
                        return Err(failure(held));
                    }
                    return Err(failure(format!(
                        "The last save of this project did not finish, and which of the files                          it left is the project cannot be told. Nothing was moved or removed.                          They are next to where the project was: {}",
                        names.join(", ")
                    )));
                }
                let mut opened =
                    project_io::open_project(&path, OpenOptions::default()).map_err(failure)?;
                // The copy's entry names the file it stands in for (where it
                // was and which file it was), and why, so that file finds it
                // again and its window can say what it is.
                if let (OpenDecision::AsCopy(reason), Value::Object(m)) = (&decision, &mut meta) {
                    m.insert(
                        "copyOf",
                        copy_record(&path, reason, opened.file.version.as_deref()),
                    );
                }
                // Recovery follows the file, not its path: unsaved work left
                // in the journal of the name or place it had before comes
                // along, with where the person left off in it. Work that
                // only may be this file's is not taken: it is offered.
                if decision == OpenDecision::Writable
                    && let Some(version) = &opened.file.version
                {
                    match self.left_behind(&id, &path, version) {
                        Some(Left::Mine(left)) => meta = left,
                        Some(Left::Maybe { entry, name }) => maybe_left = Some((entry, name)),
                        None => {}
                    }
                }
                // The last save here went to a volume that could not prove
                // it durable, and a copy of it was kept on this machine. The
                // file read back with that version confirms it; any other
                // file means that save may not have arrived, and the person
                // is told where the copy is.
                if decision == OpenDecision::Writable
                    && let Some(version) = &opened.file.version
                    && let Some(copy) = self.unconfirmed_copy(&id, &version.hash)
                    && let Value::Object(m) = &mut meta
                {
                    push_kept(m, [kept_row("unconfirmed", Some(&copy))]);
                }
                // What a save that was cut short left and recovery did not
                // move is said once the project is open, with what its
                // records tell (`ShellCall::KeptBeside`).
                if let Value::Object(m) = &mut meta {
                    push_kept(
                        m,
                        left_aside
                            .undecided
                            .iter()
                            .filter(|l| l.why != project_io::Undecided::Active)
                            .map(|l| kept_row("undecided", Some(&l.path))),
                    );
                }
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
        // Work that may be this file's is offered only where someone can be
        // asked, and only if it replays cleanly onto exactly this file.
        let mut from_other = None;
        if !journaled
            && !matches!(mode, RecoveryMode::Legacy)
            && let Some((entry, name)) = maybe_left
        {
            let document = source_document(&source);
            let saved = saved_state(&source);
            let found = self.recover_from(
                &entry,
                Some(&base),
                Some(crate::journal::Base {
                    document: &document,
                    history: saved.as_ref().map(|(h, _)| h),
                    workspace: saved.as_ref().map(|(_, w)| w),
                }),
            );
            if matches!(found, Recovery::Recovered(_)) {
                recovered = found;
                from_other = Some((entry, name));
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
            let recovery = match (&recovered, &from_other) {
                (Recovery::Recovered(r), Some((_, name))) => RecoveryResult::FromOtherFile {
                    name: name.clone(),
                    truncated: r.truncated,
                },
                _ => match &recovered {
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
                },
            };
            self.pending_recovery.retain(|_, p| p.id != id);
            let ticket = crate::new_id("recovery-choice");
            let journals =
                self.project_dir(from_other.as_ref().map_or(recovery_id, |(entry, _)| entry));
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
                from_other: from_other.map(|(entry, _)| entry),
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
                from_other: from_other.map(|(entry, _)| entry),
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
            from_other,
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
        // Work offered from another file and declined is left exactly where
        // it is: it may not be this file's to set aside.
        if setting_aside && from_other.is_none() {
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
        // What the first save would work out about a long undo history on
        // this thread is worked out beside it instead, from now.
        if let Some(p) = self.project(&id) {
            let _ = std::thread::Builder::new()
                .name("pm-save-warmup".into())
                .spawn(p.store().save_warmup());
        }
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
                unfinalized: None,
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
        if save_as.is_none() {
            self.follow_file(id);
        }
        let mut new_lock = None;
        if let Some((path, _)) = &save_as {
            let decision = locks::open_decision(path, None);
            if decision != OpenDecision::Writable {
                return Err(failure(format!(
                    "Choose a writable local destination: {decision:?}"
                )));
            }
            new_lock = Some(self.lock_project(path).map_err(failure)?);
        }
        let mut recovery_copy = None;
        let mut displaced = None;
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
                // No save point, no save: the file stays as it is.
                self.mark_save_point(id, &plan.document, plan.revision)?;
            }
            let p = self.project_mut(id).unwrap();
            let mut changed = Vec::new();
            let mut unproven = None;
            let result = plan.and_then(|plan| plan.run()).and_then(|receipt| {
                displaced = receipt.displaced.clone();
                changed = receipt.changed.clone();
                unproven = Some(receipt.unproven);
                p.store_mut().finish_save(receipt)
            });
            if let Some(unproven) = unproven {
                self.note_unproven(id, unproven);
            }
            // A retained version found changed is another version to tell
            // about: named by the reply where nothing else is.
            displaced = displaced.or_else(|| changed.into_iter().next());
            if result.is_ok() {
                self.save_published(id);
            }
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
        crate::fault_injection::hit(crate::fault_injection::Stage::InSave);
        self.rebase_journal(id)?;
        Ok(ShellReply::Saved {
            session: handle.clone(),
            recovery_copy,
            displaced,
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

/// Unsaved work a file entry left behind, as it relates to a file being
/// opened ([`Kernel::left_behind`]).
enum Left {
    /// The entry is this file: its journal was moved to it; its metadata.
    Mine(Value),
    /// The entry may be this file (its id, and the file name it had).
    Maybe { entry: String, name: String },
}
/// What a copy's entry records about the file it stands in for (`copyOf`):
/// where it was, why it opened as a copy, and which file it was (`source`:
/// its content hash, and the stat that says without reading it that it is
/// still that file).
fn copy_record(
    path: &std::path::Path,
    reason: &CopyReason,
    source: Option<&crate::project_io::FileVersion>,
) -> Value {
    let mut m = Map::new();
    m.insert("path", Value::from(path.to_string_lossy().into_owned()));
    m.insert("reason", Value::from(format!("{reason:?}")));
    if let Some(version) = source {
        let mut s = Map::new();
        // Text: these do not fit a JSON number.
        s.insert("dev", Value::from(version.stat.dev.to_string()));
        s.insert("ino", Value::from(version.stat.ino.to_string()));
        s.insert("size", Value::from(version.stat.size.to_string()));
        s.insert("mtime", Value::from(version.stat.mtime_ns.to_string()));
        s.insert("hash", Value::from(version.hash.clone()));
        m.insert("source", Value::Object(s));
    }
    Value::Object(m)
}
/// Whether the file at `path` now is the one a copy was made from: the same
/// file untouched (device, inode, size and modification time), or a file
/// with exactly its content (a sync client rewrites a file under a new
/// inode). A copy that did not record its source is never assumed to match.
fn is_copy_source(record: &Value, path: &std::path::Path) -> bool {
    let Some(source) = record.get("source") else {
        return false;
    };
    let text = |key: &str| source.get(key).and_then(Value::as_str);
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let now = crate::project_io::durable_stat_of(&meta);
    let untouched = text("dev") == Some(&now.dev.to_string())
        && text("ino") == Some(&now.ino.to_string())
        && text("size") == Some(&now.size.to_string())
        && text("mtime") == Some(&now.mtime_ns.to_string());
    untouched
        || text("hash")
            .is_some_and(|hash| crate::project_io::hash_file(path).is_ok_and(|now| now == hash))
}

impl Kernel {
    /// The library copy made of the file now at `path` when it could not be
    /// opened in place, if the library still has it (the newest, were there
    /// several). A copy made of another file that had this name is not it.
    fn copy_of(&self, path: &std::path::Path) -> Option<String> {
        let store = self.library.as_ref()?.store();
        let same = |recorded: &str| {
            let recorded = std::path::Path::new(recorded);
            recorded == path
                || recorded
                    .canonicalize()
                    .is_ok_and(|r| path.canonicalize().is_ok_and(|p| p == r))
        };
        let mut rows = store.projects().ok()?;
        rows.sort_by(|a, b| b.at.partial_cmp(&a.at).unwrap_or(std::cmp::Ordering::Equal));
        rows.into_iter().map(|row| row.id).find(|id| {
            store
                .read(&format!("projectMeta.{id}"))
                .ok()
                .flatten()
                .and_then(|meta| {
                    let record = meta.get("copyOf")?;
                    Some(same(record.get("path")?.as_str()?) && is_copy_source(record, path))
                })
                .unwrap_or(false)
        })
    }
    /// Copies in the library that hold work for the file session `id` has
    /// open in place: copies recorded for the file (`copyOf`), and copies an
    /// earlier build made of every file in a syncing folder without
    /// recording it, known by their name ("<project name> — Native"). The
    /// copies are never touched. Each is reported once per file: the file's
    /// entry remembers the ones reported (`copiesOffered`).
    ///
    /// For a session that is itself a copy of a file, the other copies
    /// recorded for that file's name: they were made of another file that
    /// had the name (it was replaced, or a link was pointed elsewhere), so
    /// this open did not take them for its own. They are reported the same
    /// way, marked `different`.
    fn unoffered_copies(&mut self, id: &str) -> Vec<Value> {
        let Some(session) = self.sessions.get(id) else {
            return Vec::new();
        };
        let (path, own_copy) = match &session.destination {
            Destination::File(path) => (path.clone(), None),
            Destination::Library(slot) => {
                let Some(of) = session
                    .meta
                    .get("copyOf")
                    .and_then(|c| c.get("path"))
                    .and_then(Value::as_str)
                else {
                    return Vec::new();
                };
                (PathBuf::from(of), Some(slot.clone()))
            }
        };
        let Some(store) = self.library.as_ref().map(|w| w.store()) else {
            return Vec::new();
        };
        let name = self
            .project(id)
            .and_then(|p| p.store().root())
            .and_then(|root| root.get("name"))
            .map(|name| name.to_value())
            .and_then(|name| name.as_str().map(|n| format!("{n} — Native")));
        let offered: Vec<String> = self
            .state
            .ui
            .get(id)
            .and_then(|ui| ui.session.get("copiesOffered"))
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let mut found = Vec::new();
        for row in store.projects().unwrap_or_default() {
            if row.id == id || own_copy.as_ref() == Some(&row.id) || offered.contains(&row.id) {
                continue;
            }
            let meta = store
                .read(&format!("projectMeta.{}", row.id))
                .ok()
                .flatten();
            let recorded = meta
                .as_ref()
                .and_then(|m| m.get("copyOf")?.get("path")?.as_str())
                .map(std::path::Path::new);
            let stands_for_it = match recorded {
                Some(recorded) => {
                    recorded == path
                        || recorded
                            .canonicalize()
                            .is_ok_and(|r| path.canonicalize().is_ok_and(|p| p == r))
                }
                // An earlier build's copy: a `copy` slot with no file of
                // its own, under the name copies are given.
                None => {
                    own_copy.is_none()
                        && row.id.starts_with("copy")
                        && meta.as_ref().is_none_or(|m| m.get("file").is_none())
                        && name.as_deref() == Some(row.name.as_str())
                }
            };
            if stands_for_it {
                let mut m = Map::new();
                m.insert("id", Value::from(row.id.clone()));
                m.insert("name", Value::from(row.name.clone()));
                if let Some(at) = row.at {
                    m.insert("at", Value::from(at));
                }
                if let Some(file) = path.file_name() {
                    m.insert("file", Value::from(file.to_string_lossy().into_owned()));
                }
                if own_copy.is_some() {
                    m.insert("different", Value::from(true));
                }
                found.push(Value::Object(m));
            }
        }
        if !found.is_empty() {
            let ids = offered
                .into_iter()
                .map(Value::from)
                .chain(found.iter().filter_map(|row| row.get("id").cloned()))
                .collect();
            if let Some(ui) = std::sync::Arc::make_mut(&mut self.state.ui).get_mut(id)
                && let Value::Object(m) = &mut ui.session
            {
                m.insert("copiesOffered", Value::Array(ids));
            }
        }
        found
    }
    /// `path` (the file `version`) is being opened under `id` and has no
    /// journal of its own. A file entry of the library that left a journal
    /// may be this file under the name or place it had before.
    ///
    /// It *is* this file when it names this path already (a save followed
    /// the file here), or when it recorded this very file: the device and
    /// inode it had when last opened or saved, which a rename or a move
    /// within the volume keep and a copy never has, with exactly these
    /// bytes. Then its journal becomes this file's and its metadata is
    /// returned ([`Left::Mine`]).
    ///
    /// It only *may* be this file when it was saved with exactly these bytes
    /// and its old path holds no file any more, but the identity does not
    /// match or was never recorded (a duplicate, a copy restored from a
    /// backup, a file moved to another volume). Equal bytes do not make two
    /// files one: nothing is moved, and the entry is returned for the person
    /// to decide ([`Left::Maybe`]).
    fn left_behind(
        &self,
        id: &str,
        path: &std::path::Path,
        version: &project_io::FileVersion,
    ) -> Option<Left> {
        let store = self.library.as_ref()?.store();
        let root = self.config_journal_root()?;
        let to = root.join(id);
        if !crate::journal::sessions(&to).is_empty() {
            return None;
        }
        let mut maybe = None;
        for row in store.projects().ok()? {
            if row.id == id || self.sessions.contains_key(&row.id) {
                continue;
            }
            let Some(meta) = store
                .read(&format!("projectMeta.{}", row.id))
                .ok()
                .flatten()
            else {
                continue;
            };
            let Some(file) = meta.get("file") else {
                continue;
            };
            let from = root.join(&row.id);
            if crate::journal::sessions(&from).is_empty() {
                continue;
            }
            let text = |key: &str| file.get(key).and_then(Value::as_str);
            let was = text("path").map(std::path::Path::new);
            let same_bytes = text("savedHash") == Some(version.hash.as_str());
            let same_file = same_bytes
                && text("dev") == Some(&version.stat.dev.to_string())
                && text("ino") == Some(&version.stat.ino.to_string());
            if was == Some(path) || same_file {
                // An empty folder left by an earlier open of this id is no journal.
                let _ = std::fs::remove_dir(&to);
                if std::fs::rename(&from, &to).is_ok() {
                    return Some(Left::Mine(meta));
                }
            } else if maybe.is_none()
                && same_bytes
                && let Some(was) = was.filter(|was| !was.is_file())
            {
                let name = was
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| row.name.clone());
                maybe = Some(Left::Maybe {
                    entry: row.id.clone(),
                    name,
                });
            }
        }
        maybe
    }
    /// What a save needs from this kernel besides the files.
    pub(crate) fn host(&self) -> project_io::Host<'_> {
        project_io::Host {
            hooks: &self.publish_hooks,
            coordinator: self.write_coordinator(),
            profile: self.profile_dir(),
            backups: None,
            backup: None,
            unconfirmed: None,
        }
    }
    /// Private files beside a project that no save's record names are never
    /// touched: they go to the host's Error Log.
    fn log_unrecorded(&mut self, settled: &project_io::Settled) {
        for path in &settled.unrecorded {
            self.log(format!(
                "A file a save left beside a project is not named by any save's record and                  was left as it is: {}",
                path.display()
            ));
        }
    }
    /// A save that failed put the version it had moved aside back under
    /// the project's name as a copy ([`ProjectIoError::PutBack`]): the same
    /// bytes in another file. The session carries on with that file, so
    /// its next save is an ordinary one, if the file there holds exactly
    /// the version the session read.
    fn adopt_put_back(&mut self, id: &str, error: &crate::ProjectIoError) {
        if !matches!(error, crate::ProjectIoError::PutBack(_)) {
            return;
        }
        let Some(Destination::File(path)) = self.sessions.get(id).map(|s| &s.destination) else {
            return;
        };
        let Some(version) = self
            .project(id)
            .and_then(|p| p.store().file().version.clone())
        else {
            return;
        };
        if let Ok(Some(now)) = project_io::FileVersion::read(path)
            && now.hash == version.hash
        {
            self.project_mut(id).unwrap().store_mut().file_moved(now);
        }
    }
    /// Whether session `id`'s last save is on a volume that could not prove
    /// it durable.
    fn note_unproven(&mut self, id: &str, unproven: bool) {
        if unproven {
            self.unproven.insert(id.to_owned());
        } else {
            self.unproven.remove(id);
        }
    }
    /// The copy this machine keeps of `id`'s last save to a volume that
    /// could not prove it durable, when the file just read (`hash`) is not
    /// that save: it may not have arrived.
    fn unconfirmed_copy(&self, id: &str, hash: &str) -> Option<PathBuf> {
        let local = self.unconfirmed_dir(id)?;
        // Read back with that version: no proof it is durable on the
        // server (it can come from a cache), so the copy stays, until a
        // later save's copy takes its place or a save is proven durable.
        if local.join(format!("{hash}.pmv")).is_file() {
            return None;
        }
        std::fs::read_dir(&local)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "pmv"))
    }
    /// The other version a save kept beside the project that its reply
    /// names: the one found at the path, else the first of the versions an
    /// earlier save had retained and this one found changed. Every one of
    /// them is in a save's record until the person has seen it
    /// (`ShellCall::KeptBeside`).
    fn note_changed(&mut self, _id: &str, receipt: &crate::SaveReceipt) -> Option<PathBuf> {
        receipt
            .displaced
            .clone()
            .or_else(|| receipt.changed.first().cloned())
    }
    /// The last save in place installed its file but its folder did not
    /// sync. Before the next save in place: the folder is synced again (a
    /// failure fails this save too, and the ticket stays), and if the name
    /// still holds exactly that save's file it is adopted, as the save it
    /// is; edits made since stay unsaved and are what this save writes. A
    /// file changed since is not adopted: this save then finds it changed.
    fn adopt_unfinalized(&mut self, id: &str) -> Result<(), KernelFailure> {
        let Some(receipt) = self.sessions.get_mut(id).and_then(|s| s.unfinalized.take()) else {
            return Ok(());
        };
        let dir = receipt
            .path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_owned();
        if let Err(e) = project_io::folder_sync_now(&dir, &self.publish_hooks) {
            self.sessions.get_mut(id).unwrap().unfinalized = Some(receipt);
            return Err(failure(crate::ProjectIoError::Finalize {
                message: e.to_string(),
                receipt: None,
            }));
        }
        if !receipt.version.is_unchanged().unwrap_or(false) {
            return Ok(());
        }
        self.note_changed(id, &receipt);
        self.note_unproven(id, receipt.unproven);
        self.project_mut(id)
            .unwrap()
            .store_mut()
            .finish_save(*receipt)
            .map_err(failure)?;
        self.save_published(id);
        Ok(())
    }
    /// Files a save or a recovery found changed when they were to go and
    /// could not keep beside their project ([`project_io::Stray`]): the
    /// Error Log says where each is (the Trash, or hidden where it was).
    /// The save's record tells the person until they have seen it; one
    /// whose record could not be written is told by the open session's own
    /// rows instead.
    fn tell_strays(&mut self) {
        let strays = match self.publish_hooks.strays.lock() {
            Ok(mut strays) => std::mem::take(&mut *strays),
            Err(_) => return,
        };
        for stray in strays {
            self.log(format!(
                "A file from an earlier save of {} was changed after it was saved and could \
                 not be kept next to the project; it is {}: {}",
                stray.project.display(),
                if stray.trashed {
                    "in the Trash"
                } else {
                    "still hidden in the project's folder"
                },
                stray.path.display()
            ));
            if stray.recorded {
                continue;
            }
            let Some(id) = self.sessions.iter().find_map(|(id, s)| {
                matches!(&s.destination, Destination::File(p) if *p == stray.project)
                    .then(|| id.clone())
            }) else {
                continue;
            };
            let why = if stray.trashed { "trashed" } else { "hidden" };
            if let Some(Value::Object(meta)) = std::sync::Arc::make_mut(&mut self.state.ui)
                .get_mut(&id)
                .map(|ui| &mut ui.session)
            {
                push_kept(meta, [kept_row(why, Some(&stray.path))]);
                let _ = self.save_session_meta(&id);
            }
        }
    }
    /// The file session `id` saves to was renamed or moved while open: carry
    /// on at the place the system says it is now (the same inode, its bytes
    /// proven unchanged, never a Trash), under a lock taken there. A file
    /// with no name any more is left to the save to report (Save As still
    /// works from the descriptor the session holds).
    fn follow_file(&mut self, id: &str) {
        let Some(Destination::File(path)) = self.sessions.get(id).map(|s| &s.destination) else {
            return;
        };
        // A save that could not put the file back under its name left it
        // aside: it is put back now, not followed to its private name.
        let path = path.clone();
        let left = project_io::settle(&path, &self.host());
        if let Some(e) = &left.unsynced {
            self.log(format!(
                "Recovery beside {} could not sync its folder: {e}",
                path.display()
            ));
        }
        self.log_unrecorded(&left);
        let Some(version) = self
            .project(id)
            .and_then(|p| p.store().file().version.clone())
        else {
            return;
        };
        let Ok(Some(moved)) = version.moved() else {
            return;
        };
        let Ok(lock) = self.lock_project(&moved.path) else {
            return;
        };
        let path = moved.path.clone();
        self.project_mut(id).unwrap().store_mut().file_moved(moved);
        let session = self.sessions.get_mut(id).unwrap();
        session.destination = Destination::File(path);
        session.lock = Some(lock);
    }
    /// Opens the copy `copy` in place of the file it stands in for; the
    /// reply still says the file opened as a copy, and why. The copy is this
    /// library's own project: what kept the file from opening in place (the
    /// original app running, a read-only folder) does not apply to it, or
    /// every open would make a copy of the copy.
    fn reopen_copy(
        &mut self,
        id: String,
        copy: String,
        decision: OpenDecision,
        mode: RecoveryMode,
    ) -> Result<ShellReply, KernelFailure> {
        let profile = self
            .library
            .as_ref()
            .ok_or_else(|| failure("Attach a native library first"))?
            .store()
            .dir()
            .to_path_buf();
        let source = SessionSource::Library {
            profile,
            project: copy,
        };
        Ok(match self.open_session_mode(id, source, None, mode)? {
            ShellReply::Opened {
                session,
                recovery,
                info,
                ..
            } => ShellReply::Opened {
                session,
                recovery,
                decision,
                info,
            },
            other => other,
        })
    }
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
        // Which file this was (text: neither fits a JSON number). A rename
        // keeps both; a copy of the file has neither.
        file.insert("dev", Value::from(version.stat.dev.to_string()));
        file.insert("ino", Value::from(version.stat.ino.to_string()));
    }
    m.insert("file", Value::Object(file));
    Value::Object(m)
}
