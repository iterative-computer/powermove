//! Immutable presentation; values come from the kernel, dates from the host.
use crate::alert::{Alert, Button};
use pm_services::ServiceErrorCode;
use std::{collections::BTreeSet, path::PathBuf};

/// Home's sections, as v1.2.0's sidebar (`legacy/ui/projects.ts`): Recents
/// and Trash. `All` is Recents without its limit (tests and searches).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    Recents,
    #[default]
    All,
    Deleted,
}
impl Section {
    /// v1's `projectsSection` value.
    pub fn key(self) -> &'static str {
        match self {
            Self::Deleted => "trash",
            _ => "recents",
        }
    }
    pub fn from_key(key: &str) -> Self {
        if key == "trash" {
            Self::Deleted
        } else {
            Self::Recents
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Deleted => "Trash",
            _ => "Recents",
        }
    }
}
/// v1's sort pop-up (`projectsSort`): Recently edited, Name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Recent,
    Name,
}
impl Sort {
    pub fn key(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Name => "name",
        }
    }
    pub fn from_key(key: &str) -> Self {
        if key == "name" {
            Self::Name
        } else {
            Self::Recent
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recently edited",
            Self::Name => "Name",
        }
    }
}
/// v1's layout toggle (`projectsView`): a grid of cards or a list of rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Grid,
    List,
}
impl Layout {
    pub fn key(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::List => "list",
        }
    }
    pub fn from_key(key: &str) -> Self {
        if key == "list" {
            Self::List
        } else {
            Self::Grid
        }
    }
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
    /// When it went to the Trash (v1's `deletedAt`).
    pub deleted_ms: Option<i64>,
    pub trashed: bool,
    pub summary: Option<Summary>,
    pub file: Option<PathBuf>,
    pub edited: bool,
    pub open: bool,
}
impl Row {
    /// v1's card line: `Edited 3 min ago · Launch Film.pmv · 1920×1080`, or
    /// `Deleted 2 d ago · 1920×1080` in the Trash. `now_ms` is the host clock.
    pub fn facts(&self, now_ms: i64) -> String {
        let dims = self.dimensions().map(|(w, h)| format!("{w}×{h}"));
        if self.trashed {
            let date = format!(
                "Deleted {}",
                ago(self.deleted_ms.or(self.modified_ms), now_ms)
            );
            return [Some(date), dims]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
        }
        let date = format!("Edited {}", ago(self.modified_ms, now_ms));
        let file = self.file.as_ref().and_then(|p| p.file_name()).map(|name| {
            let name = name.to_string_lossy().into_owned();
            if self.edited {
                format!("Unsaved changes · {name}")
            } else {
                name
            }
        });
        [Some(date), file, dims]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        let summary = self.summary.as_ref()?;
        summary
            .width
            .zip(summary.height)
            .filter(|(w, h)| *w > 0 && *h > 0)
    }
}

/// v1's `ago`: just now, N min ago, N h ago, N d ago, then the date.
pub fn ago(at_ms: Option<i64>, now_ms: i64) -> String {
    let at = at_ms.unwrap_or(0);
    let s = ((now_ms - at) as f64 / 1000.).max(1.);
    if s < 90. {
        "just now".into()
    } else if s < 3600. {
        format!("{} min ago", (s / 60.).round())
    } else if s < 86400. {
        format!("{} h ago", (s / 3600.).round())
    } else if s < 604800. {
        format!("{} d ago", (s / 86400.).round())
    } else {
        // `toLocaleDateString()` in the en-US locale v1 shipped with.
        let days = at.div_euclid(86_400_000);
        let (y, m, d) = civil(days);
        format!("{m}/{d}/{y}")
    }
}
/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
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
    pub sort: Sort,
}
impl Query {
    /// Newest first, or by name.
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
        match self.sort {
            Sort::Recent => rows.sort_by(|a, b| {
                let at = |r: &Row| r.modified_ms.or(r.deleted_ms);
                at(b).cmp(&at(a)).then_with(|| a.id.cmp(&b.id))
            }),
            Sort::Name => rows.sort_by(|a, b| {
                a.name
                    .to_lowercase()
                    .cmp(&b.name.to_lowercase())
                    .then_with(|| a.id.cmp(&b.id))
            }),
        }
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
/// v1.2.0's Home empty states (`emptyState` in `legacy/ui/projects.ts`).
pub fn empty_state(query: &Query) -> EmptyState {
    if !query.text.trim().is_empty() {
        EmptyState {
            title: "No matching projects",
            symbol: "magnifyingglass",
            detail: "Try a different name.".into(),
            action: None,
        }
    } else if query.section == Section::Deleted {
        EmptyState {
            title: "Trash is empty",
            symbol: "trash",
            detail:
                "Projects moved to Trash stay here until you restore or permanently delete them."
                    .into(),
            action: None,
        }
    } else {
        EmptyState {
            title: "No projects yet",
            symbol: "folder",
            detail:
                "Projects keep the composition, layers, reusable sections and exports together."
                    .into(),
            action: Some("Create a project"),
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
            Self::Trash => "Move to Trash…",
            Self::PutBack => "Restore",
            Self::DeleteImmediately => "Delete Forever…",
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
        format!("Move “{name}” to Trash?"),
        "You can restore this project from Trash.",
        vec![
            Button::new("Move to Trash", true).default(),
            Button::new("Cancel", false).cancel(),
        ],
    )
}
pub fn delete_alert(name: &str) -> Alert<bool> {
    Alert::new(
        format!("Delete “{name}” forever?"),
        "The project is archived in Powermove’s storage and can’t be opened from Home again.",
        vec![
            Button::new("Cancel", false).default().cancel(),
            Button::new("Delete Forever", true).destructive(),
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
    /// Preserve the kernel's diagnostic locally while retaining typed copy.
    Kernel { code: ServiceErrorCode, detail: String },
    /// Inspectable technical reason, never used as product copy.
    Unsupported(&'static str),
    Stale,
    Busy,
}
