//! Immutable presentation; values come from the kernel, dates from the host.
use crate::alert::{Alert, Button};
use pm_services::ServiceErrorCode;
use std::{collections::BTreeSet, path::PathBuf};

pub const TITLE: &str = "Project Library";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    Recents,
    #[default]
    All,
    Deleted,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub thumbnail: Option<String>,
    pub duration_seconds: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub modified_ms: Option<i64>,
    pub trashed: bool,
    pub summary: Option<Summary>,
    pub file: Option<PathBuf>,
    pub edited: bool,
    pub open: bool,
}
impl Row {
    pub fn facts(&self, relative_date: &str) -> Option<String> {
        let mut facts = vec![];
        if self.modified_ms.is_some() && !relative_date.is_empty() {
            facts.push(relative_date.to_owned());
        }
        if let Some((w, h)) = self.dimensions() {
            facts.push(format!("{w}×{h}"));
        }
        (!facts.is_empty()).then(|| facts.join(" · "))
    }
    fn dimensions(&self) -> Option<(u32, u32)> {
        let summary = self.summary.as_ref()?;
        summary
            .width
            .zip(summary.height)
            .filter(|(w, h)| *w > 0 && *h > 0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub rows: Vec<Row>,
    /// Membership is supplied by the service, not guessed from a time cutoff.
    pub recents: BTreeSet<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub section: Section,
    pub text: String,
}
impl Query {
    /// Newest first.
    pub fn apply(&self, snapshot: &Snapshot) -> Vec<Row> {
        let text = self.text.trim().to_lowercase();
        let mut rows: Vec<_> = snapshot
            .rows
            .iter()
            .filter(|row| {
                (match self.section {
                    Section::Deleted => row.trashed,
                    Section::All => !row.trashed,
                    Section::Recents => !row.trashed && snapshot.recents.contains(&row.id),
                }) && row.name.to_lowercase().contains(&text)
            })
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.modified_ms
                .cmp(&a.modified_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        // Authored v1.2.0 recents behavior: 12 until a search is entered.
        // The native spec removes pagination, not this deliberate limit.
        if self.section == Section::Recents && text.is_empty() {
            rows.truncate(12);
        }
        rows
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyState {
    pub title: &'static str,
    pub symbol: &'static str,
    pub detail: String,
    pub action: Option<&'static str>,
}
pub fn empty_state(query: &Query) -> EmptyState {
    if !query.text.trim().is_empty() {
        EmptyState {
            title: "No Results",
            symbol: "magnifyingglass",
            detail: format!("No projects match “{}”.", query.text),
            action: None,
        }
    } else if query.section == Section::Deleted {
        EmptyState {
            title: "No Deleted Projects",
            symbol: "trash",
            detail: "Projects you move to the Trash appear here.".into(),
            action: None,
        }
    } else {
        EmptyState {
            title: "No Projects",
            symbol: "square.grid.2x2",
            detail: "Projects you create appear here.".into(),
            action: Some("New Project"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Open,
    OpenNewWindow,
    Rename,
    Duplicate,
    Reveal,
    Trash,
    PutBack,
    DeleteImmediately,
}
impl MenuAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::OpenNewWindow => "Open in New Window",
            Self::Rename => "Rename…",
            Self::Duplicate => "Duplicate",
            Self::Reveal => "Show in Finder",
            Self::Trash => "Move to Trash",
            Self::PutBack => "Put Back",
            Self::DeleteImmediately => "Delete Immediately…",
        }
    }
}
/// None is a native separator. Same data drives card, row and title menu.
pub fn context_menu(row: &Row) -> Vec<Option<MenuAction>> {
    if row.trashed {
        return vec![
            Some(MenuAction::PutBack),
            None,
            Some(MenuAction::DeleteImmediately),
        ];
    }
    let mut items = vec![
        Some(MenuAction::Open),
        Some(MenuAction::OpenNewWindow),
        None,
        Some(MenuAction::Rename),
        Some(MenuAction::Duplicate),
    ];
    if row.file.is_some() {
        items.push(Some(MenuAction::Reveal));
    }
    items.extend([None, Some(MenuAction::Trash)]);
    items
}

/// Library alerts answer `true` for their action, `false` to cancel or acknowledge.
pub fn trash_alert(name: &str) -> Alert<bool> {
    Alert::new(
        format!("Move “{name}” to the Trash?"),
        "You can put it back from Recently Deleted.",
        vec![
            Button::new("Move to Trash", true).default(),
            Button::new("Cancel", false).cancel(),
        ],
    )
}
pub fn delete_alert(name: &str) -> Alert<bool> {
    Alert::new(
        format!("Delete “{name}” immediately?"),
        "The project is archived in Powermove’s storage and can’t be opened from the Library again.",
        vec![
            Button::new("Cancel", false).default().cancel(),
            Button::new("Delete", true).destructive(),
        ],
    )
    .critical()
}
pub fn error_alert(code: ServiceErrorCode, name: &str) -> Alert<bool> {
    // Use the approved generic error for a vanished, unnamed target.
    let code = if name.is_empty()
        && !matches!(code, ServiceErrorCode::Offline | ServiceErrorCode::DiskFull)
    {
        ServiceErrorCode::Unavailable
    } else {
        code
    };
    let (message, detail) = crate::surfaces::documents::service_copy(code, name);
    Alert::new(message, detail, vec![Button::new("OK", false).default()])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Service(ServiceErrorCode),
    /// Inspectable technical reason, never used as product copy.
    Unsupported(&'static str),
    Stale,
    Busy,
}
