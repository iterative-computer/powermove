//! Kernel-owned library/recents/autosave: never open a second document writer.
use crate::error::Result;
use crate::service_ensure as ensure;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub modified_ms: i64,
    pub trashed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryAction {
    Open { id: String },
    Duplicate { id: String, new_id: String },
    Archive { id: String },
    Restore { id: String },
    Checkpoint { id: String, expected_revision: u64 },
}

/// Implement by forwarding to KernelHandle on a worker. IDs and revisions belong to kernel.
pub trait LibraryHook {
    fn list(&mut self) -> Result<Vec<ProjectEntry>>;
    fn recents(&mut self) -> Result<Vec<String>>;
    fn apply(&mut self, action: LibraryAction) -> Result<()>;
}

pub struct LibraryService<H>(pub H);
impl<H: LibraryHook> LibraryService<H> {
    pub fn list(&mut self, include_trash: bool) -> Result<Vec<ProjectEntry>> {
        Ok(self
            .0
            .list()?
            .into_iter()
            .filter(|entry| include_trash || !entry.trashed)
            .collect())
    }
    /// Retain kernel recency order, skip archived/missing IDs, deduplicate without renaming.
    pub fn recents(&mut self) -> Result<Vec<ProjectEntry>> {
        let entries = self.0.list()?;
        let mut seen = std::collections::BTreeSet::new();
        Ok(self
            .0
            .recents()?
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .filter_map(|id| {
                entries
                    .iter()
                    .find(|entry| entry.id == id && !entry.trashed)
                    .cloned()
            })
            .collect())
    }
    pub fn apply(&mut self, action: LibraryAction) -> Result<()> {
        let id = match &action {
            LibraryAction::Open { id }
            | LibraryAction::Archive { id }
            | LibraryAction::Restore { id }
            | LibraryAction::Checkpoint { id, .. } => id,
            LibraryAction::Duplicate { id, new_id } => {
                ensure!(id != new_id, "duplicate needs a new id");
                super::storage::component(new_id)?;
                id
            }
        };
        super::storage::component(id)?;
        self.0.apply(action)
    }
}
