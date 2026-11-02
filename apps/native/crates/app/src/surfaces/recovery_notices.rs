//! Typed copy catalog, viewer banners/inline notices, and local Error Log.
//! Diagnostic text is deliberately absent from Notice/Banner; only LogRow has it.
use std::time::Duration;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Medium,
    High,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceCode {
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
pub struct Copy {
    pub message: String,
    pub informative: String,
}
pub fn service_copy(code: ServiceCode, name: Option<&str>) -> Copy {
    use ServiceCode::*;
    let named = name.map(|n| format!("“{n}”"));
    // No fallback object noun is invented. Omitting the optional name clause
    // follows the Services review's explicit unknown-name instruction.
    let item = named.as_deref().unwrap_or("");
    let (message, informative) = match code {
        Offline => (
            "Powermove can’t connect.".into(),
            "Check your internet connection and try again.",
        ),
        NotFound => (
            format!("{item} can’t be found.").trim_start().to_owned(),
            "It may have been moved or deleted.",
        ),
        TooLarge => (
            format!("{item} is too large to open.")
                .trim_start()
                .to_owned(),
            "",
        ),
        InvalidProject => (
            format!("{item} can’t be opened because it isn’t a Powermove project.")
                .trim_start()
                .to_owned(),
            "",
        ),
        PermissionDenied => (
            format!(
                "You don’t have permission to open{suffix}.",
                suffix = named.as_ref().map_or(String::new(), |n| format!(" {n}"))
            ),
            "Check its sharing and permissions in Finder.",
        ),
        DiskFull => (
            "There isn’t enough disk space.".into(),
            "Free some space and try again.",
        ),
        InvalidInput | InvalidData => (
            format!("{item} couldn’t be read.").trim_start().to_owned(),
            "The file may be damaged.",
        ),
        Unavailable | Unknown => (
            "The operation couldn’t be completed.".into(),
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    };
    let message = if name.is_none() {
        let mut chars = message.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().collect::<String>() + chars.as_str()
        })
    } else {
        message
    };
    Copy {
        message,
        informative: informative.into(),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub surface: &'static str,
    pub code: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    Retry,
    ShowInFinder,
    Relink,
    TurnOff,
}
impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Self::Retry => "Retry",
            Self::ShowInFinder => "Show in Finder",
            Self::Relink => "Relink…",
            Self::TurnOff => "Turn Off",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub source: Source,
    pub copy: Copy,
    pub severity: Severity,
    /// Exactly zero or one fix action, addressed to this notice's source.
    pub action: Option<Fix>,
}
impl Notice {
    pub fn priority(&self) -> Priority {
        if self.severity == Severity::Error {
            Priority::High
        } else {
            Priority::Medium
        }
    }
    pub fn placement(&self, has_viewer: bool) -> Placement {
        if has_viewer {
            Placement::ViewerBanner
        } else {
            Placement::InlineNotice
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    ViewerBanner,
    InlineNotice,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub text: String,
    pub severity: Severity,
    pub action: Option<Fix>,
    pub close_label: Option<&'static str>,
    pub dismiss_after: Option<Duration>,
}
impl Banner {
    pub fn recovered(name: &str, truncated: bool) -> Self {
        Self {
            text: format!(
                "Powermove recovered “{name}” after an unexpected quit.{}",
                if truncated {
                    " The last few seconds of edits may be missing."
                } else {
                    ""
                }
            ),
            severity: if truncated {
                Severity::Warning
            } else {
                Severity::Info
            },
            action: None,
            close_label: truncated.then_some("Dismiss"),
            dismiss_after: (!truncated).then_some(Duration::from_secs(4)),
        }
    }
    pub fn notice(notice: &Notice) -> Self {
        Self {
            text: if notice.copy.informative.is_empty() {
                notice.copy.message.clone()
            } else {
                format!("{} {}", notice.copy.message, notice.copy.informative)
            },
            severity: notice.severity,
            action: notice.action,
            close_label: notice.action.map(|_| "Dismiss"),
            dismiss_after: notice.action.is_none().then_some(Duration::from_secs(4)),
        }
    }
}
/// One visible banner per viewer. Additional notices retain arrival order.
#[derive(Default)]
pub struct BannerQueue {
    pub visible: Option<Banner>,
    pending: std::collections::VecDeque<Banner>,
    shown_at: Duration,
}
impl BannerQueue {
    pub fn push(&mut self, banner: Banner, now: Duration) {
        if self.visible.is_none() {
            self.visible = Some(banner);
            self.shown_at = now;
        } else {
            self.pending.push_back(banner);
        }
    }
    pub fn dismiss(&mut self, now: Duration) {
        self.visible = self.pending.pop_front();
        self.shown_at = now;
    }
    pub fn tick(&mut self, now: Duration) {
        if self
            .visible
            .as_ref()
            .and_then(|b| b.dismiss_after)
            .is_some_and(|delay| now.saturating_sub(self.shown_at) >= delay)
        {
            self.dismiss(now);
        }
    }
    pub fn document_closed(&mut self) {
        self.visible = None;
        self.pending.clear();
    }
}
pub const LOG_TITLE: &str = "Error Log";
pub const LOG_COLUMNS: [&str; 3] = ["Time", "Source", "Message"];
pub const LOG_DETAILS: &str = "Details";
pub const LOG_BUTTONS: [&str; 2] = ["Copy", "Clear Log"];
pub const LOG_EMPTY_TITLE: &str = "No Errors";
pub const LOG_EMPTY_LINE: &str = "Problems Powermove notices appear here.";
/// System relative short formatter consumes `time`; raw diagnostics appear only
/// in the selectable MONO_FONT Details pane and explicit Copy output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub id: String,
    pub time: std::time::SystemTime,
    pub source: Source,
    pub source_display: String,
    pub copy: Copy,
    pub diagnostic: String,
}
#[derive(Default)]
pub struct ErrorLog {
    rows: Vec<LogRow>,
    selected: std::collections::BTreeSet<String>,
    pub open: bool,
}
impl ErrorLog {
    /// A single titled NSTableView/NSSplitView window. Reopen focuses it.
    pub fn open(&mut self) {
        self.open = true;
    }
    pub fn rows(&self) -> &[LogRow] {
        &self.rows
    }
    pub fn insert(&mut self, row: LogRow) -> bool {
        if self.rows.iter().any(|r| r.id == row.id) {
            return false;
        }
        // Newest first; stable insertion order for equal instants.
        let index = self.rows.partition_point(|r| r.time > row.time);
        self.rows.insert(index, row);
        true
    }
    pub fn select(&mut self, ids: impl IntoIterator<Item = String>) {
        self.selected = ids
            .into_iter()
            .filter(|id| self.rows.iter().any(|r| &r.id == id))
            .collect();
    }
    pub fn details(&self) -> Option<&str> {
        self.rows
            .iter()
            .find(|r| self.selected.contains(&r.id))
            .map(|r| r.diagnostic.as_str())
    }
    pub fn copy_selected(
        &self,
        mut relative_short_time: impl FnMut(std::time::SystemTime) -> String,
    ) -> String {
        self.rows
            .iter()
            .filter(|r| self.selected.contains(&r.id))
            .map(|r| {
                format!(
                    "{}\t{}\t{}\n{}\n{}",
                    relative_short_time(r.time),
                    r.source_display,
                    r.copy.message,
                    r.copy.informative,
                    r.diagnostic
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    /// Diagnostic records are not user work; no confirmation is presented.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.selected.clear();
    }
}
