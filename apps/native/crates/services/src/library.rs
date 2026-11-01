//! Kernel-owned library/recents/autosave: never open a second document writer.
use crate::error::Result;
use crate::service_ensure as ensure;
use serde::{Deserialize, Serialize};

/// UI copy from design/reviews/na-cloud-services.md item 2, verbatim.
pub const MOVE_TO_TRASH_LABEL: &str = "Move to Trash";
pub const PUT_BACK_LABEL: &str = "Put Back";
pub const RECENTLY_DELETED_LABEL: &str = "Recently Deleted";

/// Kernel-owned asset identity; resolve through the host asset capability.
pub type AssetId = String;

/// Exact saved rate, including fractional rates such as 30000/1001.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

/// Display facts filled by the kernel on save; never a second project authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectSummary {
    pub thumbnail: Option<AssetId>,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    /// Pass to the system relative-date formatter (e.g. "Today, 11:42", "Yesterday").
    /// Do not cache an English date string or format dates on the service worker.
    pub modified_ms: i64,
    /// Durable internal vocabulary only: UI uses the Recently Deleted filter,
    /// Move to Trash and Put Back labels above, never "Archive" or "Restore".
    pub trashed: bool,
    /// Missing in older indexes: unknown display data, never a fabricated zero summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ProjectSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryAction {
    Open {
        id: String,
    },
    Duplicate {
        id: String,
        new_id: String,
    },
    /// Internal action name; present as Move to Trash (review item 2).
    Archive {
        id: String,
    },
    /// Internal action name; present as Put Back (review item 2).
    Restore {
        id: String,
    },
    Checkpoint {
        id: String,
        expected_revision: u64,
    },
}

impl LibraryAction {
    /// Labels specified by design; other actions keep their owning UI's copy.
    pub fn trash_menu_label(&self) -> Option<&'static str> {
        match self {
            Self::Archive { .. } => Some(MOVE_TO_TRASH_LABEL),
            Self::Restore { .. } => Some(PUT_BACK_LABEL),
            _ => None,
        }
    }
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
