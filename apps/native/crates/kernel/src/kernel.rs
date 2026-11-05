//! The kernel core: every open project, its transactions and its journal.
//!
//! [`Kernel`] is synchronous and single-threaded by construction; the
//! [`actor`](crate::actor) runs it on its own thread and is the only way the
//! app reaches it. Tests drive it directly.
//!
//! Order inside [`Kernel::handle`] (the publication boundary of §3.2.1):
//! the call runs and publishes its revision in the store, its durable
//! changes are `write(2)`n to the journal, and only then does the caller get
//! its acknowledgement (and observers their events, from the actor).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pm_model::Document;
use pm_model::json::Value;

use crate::document::DocumentStore;
use crate::history::History;
use crate::journal::{
    self, CheckpointState, Durability, Journal, JournalConfig, Recovered, Recovery,
};
use crate::library::LibraryProject;
use crate::model;
use crate::project_io::OpenedProject;
use crate::tx::{Call, Handled, Notice, Project, Request, WorkspaceCall};

/// Kernel knobs.
#[derive(Clone, Debug, Default)]
pub struct KernelConfig {
    /// Where journals live (`<root>/<project id>/<session>/`). `None`: no
    /// journal (tests, headless tools that never edit).
    pub journal_root: Option<PathBuf>,
    pub journal: JournalConfig,
}

/// Where an opened project comes from.
#[derive(Debug)]
pub enum OpenSource {
    /// A new project from a project value (stored or canonical form).
    Untitled(Value),
    /// A `.pmv` read by [`crate::open_project`].
    File(OpenedProject),
    RecoveredFile {
        opened: OpenedProject,
        supersedes: Vec<PathBuf>,
    },
    /// A library slot read by [`crate::library::LibraryStore::load`].
    Library(LibraryProject),
    LibraryWithFile {
        lib: LibraryProject,
        file: crate::project_io::ProjectFile,
    },
    /// Recovered work ([`Kernel::recover`]); `supersedes` are the sessions
    /// it replaces (deleted once the new session's checkpoint is durable).
    Recovered {
        recovered: Recovered,
        supersedes: Vec<PathBuf>,
    },
}

/// What [`Kernel::open`] reports.
#[derive(Clone, Debug)]
pub struct OpenInfo {
    pub project: String,
    pub revision: u64,
    /// Canonical SHA-256 of the document as opened (the journal base).
    pub base_hash: String,
    pub journal: Option<PathBuf>,
    /// History entries imported.
    pub history_entries: usize,
    /// Persisted history that could not be imported (never silently: the
    /// project opens with empty undo and this notice).
    pub history_rejected: bool,
    /// Why (generic wording).
    pub history_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KernelFailure {
    NoProject(String),
    AlreadyOpen(String),
    Journal(String),
    Storage(String),
}

impl std::fmt::Display for KernelFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelFailure::NoProject(id) => write!(f, "No open project {id}"),
            KernelFailure::AlreadyOpen(id) => write!(f, "Project {id} is already open"),
            KernelFailure::Storage(m) => write!(f, "{m}"),
            KernelFailure::Journal(m) => {
                write!(f, "The recovery journal could not be started: {m}")
            }
        }
    }
}

impl std::error::Error for KernelFailure {}

pub(crate) struct Slot {
    pub project: Project,
    journal: Option<Journal>,
    base_hash: String,
}

/// Every open project. See the module docs.
pub struct Kernel {
    config: KernelConfig,
    history_order: Vec<String>,
    history_max_bytes: usize,
    history_max_entries: usize,
    pub(crate) trimmed_projects: Vec<String>,
    durability: Option<Durability>,
    slots: BTreeMap<String, Slot>,
    pub(crate) compacting: bool,
    pub(crate) pending_recovery: BTreeMap<String, crate::session::PendingRecovery>,
    pub(crate) library: Option<crate::persistence::LibraryWriter>,
    pub(crate) sessions: BTreeMap<String, crate::session::Session>,
    pub(crate) state: crate::state::StateSnapshot,
}

impl Kernel {
    pub fn new(config: KernelConfig) -> Kernel {
        let durability = config
            .journal_root
            .as_ref()
            .map(|_| Durability::new(config.journal.clone()));
        Kernel {
            config,
            history_order: Vec::new(),
            history_max_bytes: crate::history::DEFAULT_MAX_BYTES,
            history_max_entries: crate::history::MAX_ENTRIES,
            trimmed_projects: Vec::new(),
            durability,
            slots: BTreeMap::new(),
            library: None,
            compacting: false,
            pending_recovery: BTreeMap::new(),
            sessions: BTreeMap::new(),
            state: Default::default(),
        }
    }

    pub fn project(&self, id: &str) -> Option<&Project> {
        self.slots.get(id).map(|s| &s.project)
    }

    pub fn project_mut(&mut self, id: &str) -> Option<&mut Project> {
        self.slots.get_mut(id).map(|s| &mut s.project)
    }

    pub fn projects(&self) -> impl Iterator<Item = &str> {
        self.slots.keys().map(String::as_str)
    }

    pub fn durability(&self) -> Option<&Durability> {
        self.durability.as_ref()
    }

    fn project_dir(&self, id: &str) -> Option<PathBuf> {
        let durable_id = self
            .sessions
            .get(id)
            .and_then(|s| match &s.destination {
                crate::session::Destination::Library(id) => Some(id.as_str()),
                _ => None,
            })
            .unwrap_or(id);
        self.config
            .journal_root
            .as_ref()
            .map(|r| r.join(durable_id))
    }

    /// Opens a project under `id` (the library id, or any unique id).
    pub fn open(&mut self, id: &str, source: OpenSource) -> Result<OpenInfo, KernelFailure> {
        if crate::library::parse_key(&format!("project.{id}")).is_none() {
            return Err(KernelFailure::Storage("Invalid project id".into()));
        }
        if self.slots.contains_key(id) {
            return Err(KernelFailure::AlreadyOpen(id.to_owned()));
        }
        let mut supersedes = Vec::new();
        let mut workspace = Value::Null;
        let mut history_error = None;
        let associated_file = match &source {
            OpenSource::LibraryWithFile { file, .. } => Some(file.clone()),
            _ => None,
        };
        let mut store = match source {
            OpenSource::Untitled(project) => DocumentStore::new_untitled(project),
            OpenSource::File(opened) => {
                workspace = opened
                    .file
                    .envelope
                    .workspace
                    .clone()
                    .unwrap_or(Value::Null);
                DocumentStore::open(opened)
            }
            OpenSource::RecoveredFile {
                opened,
                supersedes: old,
            } => {
                workspace = opened
                    .file
                    .envelope
                    .workspace
                    .clone()
                    .unwrap_or(Value::Null);
                supersedes = old;
                let mut store = DocumentStore::open(opened);
                store.mark_recovered();
                store
            }
            OpenSource::Library(lib) | OpenSource::LibraryWithFile { lib, .. } => {
                let canonical = model::to_canonical(lib.project);
                let open_comp = canonical
                    .get("compId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let history = match &lib.history {
                    None => History::new(),
                    Some(saved) => {
                        History::import(saved, open_comp.as_deref()).unwrap_or_else(|e| {
                            history_error = Some(e);
                            History::new()
                        })
                    }
                };
                if let Some(ws) = lib.workspace.or_else(|| {
                    lib.session
                        .as_ref()
                        .and_then(|m| m.get("workspace"))
                        .cloned()
                }) {
                    workspace = ws;
                }
                DocumentStore::adopt(Document::new(Some(canonical), 1), history)
            }
            OpenSource::Recovered {
                recovered,
                supersedes: old,
            } => {
                supersedes = old;
                workspace = recovered.workspace;
                DocumentStore::recovered(recovered.document, recovered.history)
            }
        };
        if let Some(file) = associated_file {
            store.inherit_file(file);
        }
        let base_hash = store
            .current()
            .root()
            .map(pm_model::hash::canonical_hash)
            .unwrap_or_default();
        if store
            .root()
            .is_some_and(|root| root.get(crate::media_versions::FIELD).is_some())
            && let Some(profile) = self
                .library
                .as_ref()
                .and_then(|writer| writer.store().dir().parent())
        {
            let archive = crate::media_archive::Archive::open(profile)
                .map_err(|e| KernelFailure::Storage(e.to_string()))?;
            store.hydrate_originals(&archive);
        }
        let mut project = Project::new(store);
        project.set_workspace(workspace.clone());
        let journal = match (&self.durability, self.project_dir(id)) {
            (Some(durability), Some(dir)) => {
                project.store_mut().enable_tap();
                let store = project.store();
                let initial = CheckpointState {
                    revision: store.revision(),
                    document: store.current().clone(),
                    history: store.history().snapshot(),
                    workspace,
                };
                Some(
                    Journal::create(durability, &dir, id, &base_hash, initial, supersedes)
                        .map_err(|e| KernelFailure::Journal(e.to_string()))?,
                )
            }
            _ => None,
        };
        let info = OpenInfo {
            project: id.to_owned(),
            revision: project.store().revision(),
            base_hash: base_hash.clone(),
            journal: journal.as_ref().map(|j| j.dir().to_path_buf()),
            history_entries: project.store().history().len(),
            history_rejected: history_error.is_some(),
            history_error,
        };
        self.slots.insert(
            id.to_owned(),
            Slot {
                project,
                journal,
                base_hash,
            },
        );
        self.maintain_history(id);
        Ok(info)
    }

    /// Closes a project. Its journal is flushed and **kept**: until the
    /// library write path compacts it into the slot, it is the work.
    pub fn close(&mut self, id: &str) -> bool {
        if let Some(s) = self.sessions.get(id) {
            let handle = s.handle.clone();
            return self.shell(crate::session::ShellCall::Close(handle)).is_ok();
        }
        if self.project(id).is_some_and(|p| p.store().is_saving())
            || self.flush_project(id).is_err()
        {
            return false;
        }
        self.remove_closed_project(id)
    }
    /// Remove only after the caller's flush/compaction barrier succeeded.
    pub(crate) fn remove_closed_project(&mut self, id: &str) -> bool {
        self.sessions.remove(id);
        std::sync::Arc::make_mut(&mut self.state.ui).remove(id);
        self.history_order.retain(|p| p != id);
        self.slots.remove(id).is_some()
    }

    pub(crate) fn config_journal_root(&self) -> Option<PathBuf> {
        self.config.journal_root.clone()
    }
    pub fn journal_dir(&self, id: &str) -> Option<PathBuf> {
        self.slots
            .get(id)?
            .journal
            .as_ref()
            .map(|j| j.dir().to_path_buf())
    }
    /// The journal base of an open project.
    pub fn base_hash(&self, id: &str) -> Option<&str> {
        self.slots.get(id).map(|s| s.base_hash.as_str())
    }

    /// Recovers `id`'s newest journal session (see [`journal::recover`]).
    pub fn recover(
        &self,
        id: &str,
        current_base: Option<&str>,
        base_document: Option<&Value>,
    ) -> Recovery {
        // Recovery must not read checkpoints while their writer is publishing
        // or pruning them, nor move a session still referenced by a queued job.
        if let Some(d) = &self.durability {
            d.wait_idle();
        }
        match self.project_dir(id) {
            Some(dir) => journal::recover(&dir, current_base, base_document),
            None => Recovery::Nothing,
        }
    }

    /// Apply Memory.trim's shared budget. Background histories, oldest
    /// recently active first, surrender distant redo then undo head before
    /// the active history. Every project keeps its latest entry.
    pub fn set_history_budget(&mut self, max_entries: usize, max_bytes: usize) {
        self.history_max_entries = max_entries;
        self.history_max_bytes = max_bytes;
        if let Some(active) = self.history_order.last().cloned() {
            self.maintain_history(&active);
        }
    }

    fn maintain_history(&mut self, active: &str) {
        self.history_order
            .retain(|p| p != active && self.slots.contains_key(p));
        self.history_order.push(active.to_owned());
        let (mut entries, mut bytes) = self.slots.values().fold((0, 0), |(n, b), s| {
            let stats = s.project.store().history().stats();
            (n + stats.entries, b + stats.bytes)
        });
        for id in &self.history_order {
            let Some(slot) = self.slots.get_mut(id) else {
                continue;
            };
            while entries > self.history_max_entries || bytes > self.history_max_bytes {
                let before = slot.project.store().history().stats();
                if !slot.project.store_mut().trim_history_one() {
                    break;
                }
                let after = slot.project.store().history().stats();
                entries -= before.entries - after.entries;
                bytes -= before.bytes - after.bytes;
                if !self.trimmed_projects.contains(id) {
                    self.trimmed_projects.push(id.clone());
                }
            }
            slot.project.collect_retired();
        }
    }

    /// Runs one call on a project.
    pub fn handle(&mut self, id: &str, request: Request) -> Result<Handled, KernelFailure> {
        if let Call::AdmitMedia(admission) = &request.call {
            let profile = self
                .library
                .as_ref()
                .and_then(|writer| writer.store().dir().parent())
                .ok_or_else(|| {
                    KernelFailure::Storage(
                        "Media admission requires an attached native profile".into(),
                    )
                })?;
            let root = profile
                .canonicalize()
                .map_err(|e| KernelFailure::Storage(e.to_string()))?
                .join("media/retained");
            if admission.bindings.iter().any(|binding| {
                binding.original.root() != root
                    || binding
                        .previous
                        .as_ref()
                        .is_some_and(|prior| prior.root() != root)
            }) {
                return Err(KernelFailure::Storage(
                    "Original receipts belong to another native profile".into(),
                ));
            }
        }
        let slot = self
            .slots
            .get_mut(id)
            .ok_or_else(|| KernelFailure::NoProject(id.to_owned()))?;
        slot.project
            .store_mut()
            .set_runtime_state(self.state.clone());
        let metadata_write = matches!(&request.call, Call::Memory(_) | Call::AdmitMedia(_));
        if metadata_write && (slot.journal.is_none() || self.durability.is_none()) {
            return Err(KernelFailure::Storage(
                "Project metadata requires durable journal admission".into(),
            ));
        }
        let workspace_write = matches!(request.call, Call::Workspace(WorkspaceCall::Mutate { .. }));
        let mut handled = slot.project.handle(request);
        if let (Some(journal), Some(durability)) = (&mut slot.journal, &self.durability) {
            let taps = slot.project.store_mut().take_tap();
            let project = &slot.project;
            let workspace = workspace_write.then(|| project.workspace().clone());
            let result = journal.append(durability, &taps, workspace.as_ref(), || {
                let store = project.store();
                CheckpointState {
                    revision: store.revision(),
                    document: store.current().clone(),
                    history: store.history().snapshot(),
                    workspace: project.workspace().clone(),
                }
            });
            let result = result.and_then(|()| {
                if metadata_write {
                    journal.sync_now().and_then(|()| durability.barrier())
                } else {
                    Ok(())
                }
            });
            if let Err(e) = result {
                // Never silent: the edit is applied but not yet durable.
                let notice = Notice::NotDurable {
                    message: e.to_string(),
                };
                if let Some(ack) = &mut handled.reply
                    && ack.notice.is_none()
                {
                    ack.notice = Some(notice.clone());
                }
                slot.project.push_notice(notice);
            }
        }
        if workspace_write {
            let ws = self.project(id).unwrap().workspace().clone();
            if let Some(ws_id) = ws.get("id").and_then(Value::as_str) {
                let mut all = self
                    .state
                    .settings
                    .get("workspaces")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if let Some(existing) = all
                    .iter_mut()
                    .find(|w| w.get("id").and_then(Value::as_str) == Some(ws_id))
                {
                    *existing = ws.clone();
                } else {
                    all.push(ws.clone());
                }
                let tx = crate::state::StateTransaction {
                    expected_revision: None,
                    ops: vec![
                        crate::state::StateOp::Setting {
                            key: "workspaces".into(),
                            value: Some(Value::Array(all)),
                        },
                        crate::state::StateOp::Setting {
                            key: "workspace".into(),
                            value: Some(Value::from(ws_id)),
                        },
                    ],
                };
                if let Err(e) = self.state_transaction(tx) {
                    let notice = Notice::NotDurable {
                        message: e.to_string(),
                    };
                    if let Some(ack) = &mut handled.reply {
                        ack.notice = Some(notice.clone());
                    }
                    self.project_mut(id).unwrap().push_notice(notice);
                }
            }
        }
        self.project_mut(id).unwrap().retain_final_acks(&handled);
        self.maintain_history(id);
        Ok(handled)
    }

    /// Start a new journal base only after its library/file save barrier.
    pub(crate) fn rebase_journal(&mut self, id: &str) -> Result<(), KernelFailure> {
        let dir = self.project_dir(id);
        let library_hash = if let Some(crate::session::Session {
            destination: crate::session::Destination::Library(slot),
            ..
        }) = self.sessions.get(id)
        {
            let p = self
                .library
                .as_ref()
                .ok_or_else(|| KernelFailure::Storage("No library".into()))?
                .store()
                .load(slot)
                .map_err(|e| KernelFailure::Storage(e.to_string()))?
                .ok_or_else(|| KernelFailure::Storage("Saved slot is missing".into()))?;
            Some(base_hash_of(&p.project))
        } else {
            None
        };
        let slot = self
            .slots
            .get_mut(id)
            .ok_or_else(|| KernelFailure::NoProject(id.into()))?;
        let hash = library_hash.unwrap_or_else(|| {
            slot.project
                .store()
                .current()
                .root()
                .map(pm_model::hash::canonical_hash)
                .unwrap_or_default()
        });
        if let (Some(d), Some(dir)) = (&self.durability, dir) {
            if let Some(j) = &slot.journal {
                j.sync_now()
                    .map_err(|e| KernelFailure::Journal(e.to_string()))?;
            }
            let store = slot.project.store();
            let state = CheckpointState {
                revision: store.revision(),
                document: store.current().clone(),
                history: store.history().snapshot(),
                workspace: slot.project.workspace().clone(),
            };
            // All old sessions are kept until the new checkpoint is durable.
            let mut old = journal::sessions(&dir);
            if let Some(j) = &slot.journal
                && !old.contains(&j.dir().to_path_buf())
            {
                old.push(j.dir().to_path_buf());
            }
            let j = Journal::create(d, &dir, id, &hash, state, old)
                .map_err(|e| KernelFailure::Journal(e.to_string()))?;
            slot.journal = Some(j);
        }
        slot.base_hash = hash;
        Ok(())
    }
    pub fn state(&self) -> &crate::state::StateSnapshot {
        &self.state
    }
    pub fn state_transaction(
        &mut self,
        tx: crate::state::StateTransaction,
    ) -> Result<crate::state::StateAck, KernelFailure> {
        for op in &tx.ops {
            match op {
                crate::state::StateOp::Ui { project, .. }
                | crate::state::StateOp::UiDocument { project, .. } => {
                    let slot = self
                        .slots
                        .get(project)
                        .ok_or_else(|| KernelFailure::NoProject(project.clone()))?;
                    if let crate::state::StateOp::UiDocument { document, .. } = op
                        && *document != slot.project.store().doc_id()
                    {
                        return Err(KernelFailure::Storage(
                            "The document was closed or replaced; UI state was not changed".into(),
                        ));
                    }
                }
                _ => {}
            }
        }
        let (next, changes) = self.state.stage(tx).map_err(KernelFailure::Storage)?;
        for change in &changes {
            if let crate::state::StateChange::Setting {
                key,
                value: Some(value),
            } = change
                && pm_model::json::to_canonical_string(value).len()
                    > crate::library::value_limit(key)
            {
                return Err(KernelFailure::Storage(
                    "Settings value exceeds its size limit".into(),
                ));
            }
        }
        if let Some(writer) = &self.library
            && changes.iter().any(|c| {
                matches!(
                    c,
                    crate::state::StateChange::Setting { .. }
                        | crate::state::StateChange::Extensions
                )
            })
        {
            // Durable state intent before updating legacy slots. On restart this
            // rolls the settings transaction forward, including deletions.
            let bytes = pm_model::json::to_canonical_string(&next.durable_value());
            crate::persistence::atomic_write(
                &writer
                    .store()
                    .dir()
                    .parent()
                    .unwrap()
                    .join("kernel-state.json"),
                bytes.as_bytes(),
            )
            .map_err(|e| KernelFailure::Storage(e.to_string()))?;
            for change in &changes {
                if let crate::state::StateChange::Setting { key, value } = change {
                    match value {
                        Some(v) => writer.set(key, v.clone()),
                        None => writer.delete(key),
                    }
                    .map_err(|e| KernelFailure::Storage(e.to_string()))?;
                }
            }
            writer
                .flush()
                .map_err(|e| KernelFailure::Storage(e.to_string()))?;
        }
        self.state = next;
        Ok(crate::state::StateAck {
            revision: self.state.revision,
            changes,
        })
    }

    /// Preserve rejected work after finishing every writer that uses its path.
    pub(crate) fn archive_journal(&mut self, id: &str) -> Result<(), KernelFailure> {
        self.flush_project(id)?;
        let journal = self
            .slots
            .get_mut(id)
            .unwrap()
            .journal
            .as_mut()
            .ok_or_else(|| {
                KernelFailure::Journal(
                    "DiscardClose requires a journal to preserve rejected work".into(),
                )
            })?;
        journal
            .archive()
            .map_err(|e| KernelFailure::Journal(e.to_string()))
    }

    /// Finish checkpoint work and fsync all segments before releasing a session.
    pub fn flush_project(&self, id: &str) -> Result<(), KernelFailure> {
        let slot = self
            .slots
            .get(id)
            .ok_or_else(|| KernelFailure::NoProject(id.into()))?;
        if let Some(journal) = &slot.journal {
            journal
                .sync_now()
                .map_err(|e| KernelFailure::Journal(e.to_string()))?;
        }
        if let Some(d) = &self.durability {
            d.barrier()
                .map_err(|e| KernelFailure::Journal(e.to_string()))?;
        }
        Ok(())
    }
    pub fn durability_barrier(&self) -> Result<(), KernelFailure> {
        for id in self.projects() {
            self.flush_project(id)?;
        }
        if let Some(writer) = &self.library {
            writer
                .flush()
                .map_err(|e| KernelFailure::Storage(e.to_string()))?;
        }
        if let Some(d) = &self.durability {
            d.barrier()
                .map_err(|e| KernelFailure::Journal(e.to_string()))?;
        }
        Ok(())
    }

    /// Best-effort flush for Drop; callers certifying shutdown use durability_barrier.
    pub fn sync_all(&self) {
        if let Some(writer) = &self.library {
            let _ = writer.flush();
        }
        for slot in self.slots.values() {
            if let Some(j) = &slot.journal {
                let _ = j.sync_now();
            }
        }
        if let Some(d) = &self.durability {
            d.wait_idle();
        }
    }
}

/// The canonical hash the journal uses for a project value (stored or
/// canonical form): pass it as `current_base` to [`Kernel::recover`].
pub fn base_hash_of(project: &Value) -> String {
    pm_model::hash::canonical_hash(&model::to_canonical(project.clone()))
}

#[allow(dead_code)]
fn _paths(_: &Path) {}
