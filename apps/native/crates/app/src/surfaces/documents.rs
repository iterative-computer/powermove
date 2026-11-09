//! S002/S004 document alerts, design/surfaces/documents.md: closing with
//! unsaved changes, quitting with several, and a project that can't open.
//! `lifecycle.rs` owns the close and quit sequence.
use crate::alert::{Alert, Button, Key};
use pm_services::ServiceErrorCode;

#[cfg(test)]
#[path = "documents_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Save,
    Cancel,
    DontSave,
    ReviewChanges,
    DiscardChanges,
    ShowInFinder,
    Ok,
}
pub fn unsaved(name: &str) -> Alert<Action> {
    Alert::new(
        format!("Save changes to “{name}” before closing?"),
        "Your project file hasn’t been updated. Local recovery is kept separately.",
        vec![
            Button::new("Save", Action::Save).default(),
            Button::new("Cancel", Action::Cancel).cancel(),
            Button::new("Don’t Save", Action::DontSave).key(Key::CommandD),
        ],
    )
}
pub fn review_quit(count: usize) -> Alert<Action> {
    assert!(count >= 2);
    Alert::new(
        format!(
            "You have {count} projects with unsaved changes. Do you want to review these changes before quitting?"
        ),
        "If you discard them, their project files aren’t updated. Local recovery is kept separately.",
        vec![
            Button::new("Review Changes…", Action::ReviewChanges).default(),
            Button::new("Cancel", Action::Cancel).cancel(),
            Button::new("Discard Changes", Action::DiscardChanges),
        ],
    )
}
/// The copy for a service failure on `name`. Diagnostics never become copy.
pub fn service_copy(code: ServiceErrorCode, name: &str) -> (String, &'static str) {
    use ServiceErrorCode::*;
    match code {
        Offline => (
            "Powermove can’t connect.".into(),
            "Check your internet connection and try again.",
        ),
        NotFound => (
            format!("“{name}” can’t be found."),
            "It may have been moved or deleted.",
        ),
        TooLarge => (format!("“{name}” is too large to open."), ""),
        InvalidProject => (
            format!("“{name}” can’t be opened because it isn’t a Powermove project."),
            "",
        ),
        PermissionDenied => (
            format!("You don’t have permission to open “{name}”."),
            "Check its sharing and permissions in Finder.",
        ),
        DiskFull => (
            "There isn’t enough disk space.".into(),
            "Free some space and try again.",
        ),
        InvalidInput | InvalidData => (
            format!("“{name}” couldn’t be read."),
            "The file may be damaged.",
        ),
        Unavailable | Unknown => (
            "The operation couldn’t be completed.".into(),
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    }
}
/// Why a project didn't open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenFailure {
    Service(ServiceErrorCode),
    /// The recovery journal that protects unsaved work couldn't start.
    Journal,
}
pub fn open_error(name: &str, failure: OpenFailure, path_exists: bool) -> Alert<Action> {
    let (message, informative) = match failure {
        OpenFailure::Service(code) => service_copy(code, name),
        OpenFailure::Journal => (
            format!("“{name}” couldn’t be opened."),
            "Powermove couldn’t start the recovery journal that protects unsaved work. Check free space and permissions in your library, then try again.",
        ),
    };
    // S004 makes OK the default even though Show in Finder is written first.
    let mut buttons = vec![Button::new("OK", Action::Ok).default()];
    if path_exists {
        buttons.push(Button::new("Show in Finder", Action::ShowInFinder));
    }
    Alert::new(message, informative, buttons)
}
