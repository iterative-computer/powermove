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
    /// A library slot read by [`crate::library::LibraryStore::load`].
    Library(LibraryProject),
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
}

impl std::fmt::Display for KernelFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelFailure::NoProject(id) => write!(f, "No open project {id}"),
            KernelFailure::AlreadyOpen(id) => write!(f, "Project {id} is already open"),
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
    durability: Option<Durability>,
    slots: BTreeMap<String, Slot>,
}

impl Kernel {
    pub fn new(config: KernelConfig) -> Kernel {
        let durability = config
            .journal_root
            .as_ref()
            .map(|_| Durability::new(config.journal.clone()));
        Kernel {
            config,
            durability,
            slots: BTreeMap::new(),
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
        self.config.journal_root.as_ref().map(|r| r.join(id))
    }

    /// Opens a project under `id` (the library id, or any unique id).
    pub fn open(&mut self, id: &str, source: OpenSource) -> Result<OpenInfo, KernelFailure> {
        if self.slots.contains_key(id) {
            return Err(KernelFailure::AlreadyOpen(id.to_owned()));
        }
        let mut supersedes = Vec::new();
        let mut workspace = Value::Null;
        let mut history_error = None;
        let store = match source {
            OpenSource::Untitled(project) => DocumentStore::new_untitled(project),
            OpenSource::File(opened) => DocumentStore::open(opened),
            OpenSource::Library(lib) => {
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
                if let Some(ws) = lib.workspace {
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
        let base_hash = store
            .current()
            .root()
            .map(pm_model::hash::canonical_hash)
            .unwrap_or_default();
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
        Ok(info)
    }

    /// Closes a project. Its journal is flushed and **kept**: until the
    /// library write path compacts it into the slot, it is the work.
    pub fn close(&mut self, id: &str) -> bool {
        match self.slots.remove(id) {
            Some(slot) => {
                if let Some(j) = &slot.journal {
                    let _ = j.sync_now();
                }
                true
            }
            None => false,
        }
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
        match self.project_dir(id) {
            Some(dir) => journal::recover(&dir, current_base, base_document),
            None => Recovery::Nothing,
        }
    }

    /// Runs one call on a project.
    pub fn handle(&mut self, id: &str, request: Request) -> Result<Handled, KernelFailure> {
        let slot = self
            .slots
            .get_mut(id)
            .ok_or_else(|| KernelFailure::NoProject(id.to_owned()))?;
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
        Ok(handled)
    }

    /// Flushes every journal now (clean shutdown, tests).
    pub fn sync_all(&self) {
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
