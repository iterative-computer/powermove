//! Immutable presentation; values come from the kernel, dates from the host.
use std::{collections::BTreeSet, ops::Range, path::PathBuf};

pub const TITLE: &str = "Project Library";
pub const SEARCH: &str = "Search Projects";
pub const COLUMNS: &[&str] = &["Name", "Date Modified", "Duration", "Size", "Frame Rate"];
pub const SIDEBAR: &[(&str, &str)] = &[
    ("Recents", "clock"),
    ("All Projects", "square.grid.2x2"),
    ("Recently Deleted", "trash"),
];
pub const VIEW_BUTTONS: &[(&str, &str)] = &[
    ("Grid View", "square.grid.2x2"),
    ("List View", "list.bullet"),
];
pub const SORT_OPTIONS: &[&str] = &["Date Modified", "Name"];

/// Host uses these exact widgets, not browser controls or replacement dialogs.
pub const NATIVE_PIECES: &[&str] = &[
    "NSWindow",
    "NSToolbar",
    "NSSplitViewController",
    "NSSegmentedControl",
    "NSSearchToolbarItem",
    "NSTableView",
    "NSPopUpButton",
    "NSTextField",
    "NSAlert.beginSheetModal",
    "NSWindow.beginSheet",
    "NSWindow.titleMenu",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    Recents,
    #[default]
    All,
    Deleted,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Grid,
    List,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Modified,
    Name,
    Duration {
        ascending: bool,
    },
    Size {
        ascending: bool,
    },
    FrameRate {
        ascending: bool,
    },
}
impl Sort {
    pub fn popup_check(self) -> Option<usize> {
        match self {
            Self::Modified => Some(0),
            Self::Name => Some(1),
            _ => None,
        }
    }
    /// S017 table header input; numeric columns start descending and reverse
    /// only when the same header is clicked again. Work remains on the worker.
    pub fn click_header(self, label: &str) -> Option<Self> {
        let next = match label {
            "Name" => Self::Name,
            "Date Modified" => Self::Modified,
            "Duration" => Self::Duration { ascending: false },
            "Size" => Self::Size { ascending: false },
            "Frame Rate" => Self::FrameRate { ascending: false },
            _ => return None,
        };
        Some(
            if std::mem::discriminant(&self) == std::mem::discriminant(&next) {
                self.reversed()
            } else {
                next
            },
        )
    }
    pub fn reversed(self) -> Self {
        match self {
            Self::Duration { ascending } => Self::Duration {
                ascending: !ascending,
            },
            Self::Size { ascending } => Self::Size {
                ascending: !ascending,
            },
            Self::FrameRate { ascending } => Self::FrameRate {
                ascending: !ascending,
            },
            other => other,
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
    pub trashed: bool,
    pub summary: Option<Summary>,
    pub file: Option<PathBuf>,
    pub edited: bool,
    pub open: bool,
}
impl Row {
    pub fn file_tooltip(&self) -> String {
        self.file.as_ref().map_or_else(
            || "Not saved to a file".into(),
            |p| format!("Saved to {}", p.display()),
        )
    }
    pub fn edited_label(&self) -> Option<&'static str> {
        self.edited.then_some("Edited")
    }
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
    /// Keep every known fact, including the name when no summary is available.
    pub fn ax_label(&self, relative_date: &str) -> Option<String> {
        let mut facts = vec![self.name.clone()];
        if self.modified_ms.is_some() && !relative_date.is_empty() {
            facts.push(format!("edited {relative_date}"));
        }
        if let Some((w, h)) = self.dimensions() {
            facts.push(format!("{w} by {h}"));
        }
        Some(facts.join(", "))
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
    pub sort: Sort,
    pub text: String,
}
impl Query {
    /// Worker only, including for >2,000 projects. No pagination.
    pub fn apply(&self, snapshot: &Snapshot) -> Vec<Row> {
        self.apply_with_names(snapshot, |a, b| a.to_lowercase().cmp(&b.to_lowercase()))
    }
    /// The native worker supplies system locale collation. The deterministic
    /// headless comparator above is never used as a macOS collation substitute.
    pub fn apply_with_names(
        &self,
        snapshot: &Snapshot,
        compare_names: impl Fn(&str, &str) -> std::cmp::Ordering,
    ) -> Vec<Row> {
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
        rows.sort_by(|a, b| match self.sort {
            Sort::Modified => b
                .modified_ms
                .cmp(&a.modified_ms)
                .then_with(|| a.id.cmp(&b.id)),
            Sort::Name => compare_names(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)),
            Sort::Duration { ascending } => compare_fact(a, b, ascending, |r| {
                r.summary
                    .as_ref()?
                    .duration_seconds
                    .filter(|v| v.is_finite() && *v >= 0.)
            }),
            Sort::FrameRate { ascending } => compare_fact(a, b, ascending, |r| {
                r.summary.as_ref()?.fps.filter(|v| v.is_finite() && *v > 0.)
            }),
            Sort::Size { ascending } => {
                let size = |r: &Row| {
                    r.dimensions()
                        .map(|(w, h)| (u64::from(w) * u64::from(h), w))
                };
                let order = match (size(a), size(b)) {
                    (Some(a), Some(b)) => {
                        if ascending {
                            a.cmp(&b)
                        } else {
                            b.cmp(&a)
                        }
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                };
                order.then_with(|| a.id.cmp(&b.id))
            }
        });
        // Authored v1.2.0 recents behavior: 12 until a search is entered.
        // The native spec removes pagination, not this deliberate limit.
        if self.section == Section::Recents && text.is_empty() {
            rows.truncate(12);
        }
        rows
    }
}

fn compare_fact(
    a: &Row,
    b: &Row,
    ascending: bool,
    fact: impl Fn(&Row) -> Option<f64>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let order = match (fact(a), fact(b)) {
        (Some(a), Some(b)) => {
            if ascending {
                a.total_cmp(&b)
            } else {
                b.total_cmp(&a)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        _ => Ordering::Equal,
    };
    order.then_with(|| a.id.cmp(&b.id))
}
pub const LOADING_AX: &str = "Loading Projects";
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub message: String,
    pub detail: String,
    pub buttons: Vec<&'static str>,
    pub default: usize,
    pub destructive: Option<usize>,
    pub critical: bool,
}
impl Alert {
    pub fn trash(name: &str) -> Self {
        Self {
            message: format!("Move “{name}” to the Trash?"),
            detail: "You can put it back from Recently Deleted.".into(),
            buttons: vec!["Move to Trash", "Cancel"],
            default: 0,
            destructive: None,
            critical: false,
        }
    }
    pub fn delete(name: &str) -> Self {
        Self {
        message: format!("Delete “{name}” immediately?"),
        detail: "The project is archived in Powermove’s storage and can’t be opened from the Library again.".into(),
        buttons: vec!["Cancel", "Delete"], default: 0, destructive: Some(1), critical: true,
    }
    }
    pub fn save_in_trash(name: &str) -> Self {
        Self {
            message: format!("“{name}” is in the Trash."),
            detail: String::new(),
            buttons: vec!["Put Back and Save", "Save As…", "Cancel"],
            default: 0,
            destructive: None,
            critical: false,
        }
    }
    pub fn error(code: ErrorCode, name: &str) -> Self {
        // Use the approved generic error for a vanished, unnamed target.
        let code = if name.is_empty() && !matches!(code, ErrorCode::Offline | ErrorCode::DiskFull) {
            ErrorCode::Unavailable
        } else {
            code
        };
        let item = format!("“{name}”");
        let (message, detail) = match code {
            ErrorCode::Offline => (
                "Powermove can’t connect.".into(),
                "Check your internet connection and try again.",
            ),
            ErrorCode::NotFound => (
                format!("{item} can’t be found."),
                "It may have been moved or deleted.",
            ),
            ErrorCode::TooLarge => (format!("{item} is too large to open."), ""),
            ErrorCode::InvalidProject => (
                format!("{item} can’t be opened because it isn’t a Powermove project."),
                "",
            ),
            ErrorCode::PermissionDenied => (
                format!("You don’t have permission to open {item}."),
                "Check its sharing and permissions in Finder.",
            ),
            ErrorCode::DiskFull => (
                "There isn’t enough disk space.".into(),
                "Free some space and try again.",
            ),
            ErrorCode::InvalidInput | ErrorCode::InvalidData => (
                format!("{item} couldn’t be read."),
                "The file may be damaged.",
            ),
            ErrorCode::Unavailable | ErrorCode::Unknown => (
                "The operation couldn’t be completed.".into(),
                "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
            ),
        };
        Self {
            message,
            detail: detail.into(),
            buttons: vec!["OK"],
            default: 0,
            destructive: None,
            critical: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    Offline,
    NotFound,
    TooLarge,
    InvalidProject,
    PermissionDenied,
    DiskFull,
    InvalidInput,
    InvalidData,
    Unavailable,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Service(ErrorCode),
    /// Inspectable technical reason, never used as product copy.
    Unsupported(&'static str),
    Stale,
    Busy,
}

/// One viewport of overscan on each side, arithmetic on fixed token heights.
pub fn virtual_range(
    total: usize,
    first_row: usize,
    viewport_rows: usize,
    columns: usize,
) -> Range<usize> {
    let columns = columns.max(1);
    if first_row.saturating_mul(columns) >= total {
        return total..total;
    }
    let start = first_row
        .saturating_sub(viewport_rows)
        .saturating_mul(columns)
        .min(total);
    let end = first_row
        .saturating_add(viewport_rows.saturating_mul(2))
        .saturating_mul(columns)
        .min(total);
    start..end.max(start)
}

/// Every resolved value is read from pm-design; missing fields stay symbolic.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowMetrics {
    pub min_width: f32,
    pub min_height: f32,
    pub sidebar: f32,
    pub sheet_width: f32,
    pub row: f32,
    pub radius: f32,
    pub thumbnail_min: f32,
    pub sheet_inset: f32,
    pub sheet_value_column: f32,
    pub media_badge: u32,
}
impl WindowMetrics {
    pub fn resolve(tokens: &pm_design::Tokens) -> Self {
        Self {
            min_width: tokens.metrics.min_width,
            min_height: tokens.metrics.min_height / 1.25,
            sidebar: tokens.metrics.settings_sidebar,
            sheet_width: tokens.metrics.sheet_width,
            row: tokens.metrics.row,
            radius: tokens.metrics.control_radius,
            thumbnail_min: tokens.metrics.thumbnail_min,
            sheet_inset: tokens.metrics.sheet_inset,
            sheet_value_column: tokens.metrics.sheet_value_column,
            media_badge: tokens.colors.media_badge,
        }
    }
}
