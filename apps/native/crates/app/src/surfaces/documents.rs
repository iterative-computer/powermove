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
    SaveAs,
    Ok,
    Reopen,
    Later,
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
/// An internal error interrupted an edit and the kernel stopped the project
/// (nothing half done is kept or saved); reopening recovers its journal.
pub fn stopped(name: &str) -> Alert<Action> {
    Alert::new(
        format!("“{name}” stopped after an internal error."),
        "So nothing half done is kept, Powermove stopped editing and saving it. Reopen it to recover every change made before the error. Window ▸ Error Log has the details.",
        vec![
            Button::new("Reopen", Action::Reopen).default(),
            Button::new("Later", Action::Later).cancel(),
        ],
    )
    .notice()
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
/// Why a write the person asked for (a save, the library at quit) did not
/// happen. The kernel's text only chooses the sentence: diagnostics never
/// become copy, they go to the Error Log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteFailure {
    DiskFull,
    NotAllowed,
    ReadOnlyDisk,
    Missing,
    ChangedElsewhere,
    /// The disk has no way to replace a file without risking another
    /// version of it, so Save leaves the file alone there.
    NoSafeReplace,
    InProgress,
    Other,
}
impl WriteFailure {
    pub fn of(detail: &str) -> Self {
        let t = detail.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
        if has(&["no space left", "os error 28", "quota"]) {
            Self::DiskFull
        } else if has(&["read-only file system", "os error 30"]) {
            Self::ReadOnlyDisk
        } else if has(&["permission denied", "not permitted", "os error 13"]) {
            Self::NotAllowed
        } else if has(&["cannot replace a file safely"]) {
            Self::NoSafeReplace
        } else if has(&["changed outside powermove", "other version was preserved"]) {
            Self::ChangedElsewhere
        } else if has(&["no such file", "os error 2)", "not found"]) {
            Self::Missing
        } else if has(&["already in progress", "save is in flight"]) {
            Self::InProgress
        } else {
            Self::Other
        }
    }
    pub fn sentence(self) -> &'static str {
        match self {
            Self::DiskFull => "The disk is full.",
            Self::NotAllowed => "Powermove isn’t allowed to write there.",
            Self::ReadOnlyDisk => "The disk can’t be written to.",
            Self::Missing => {
                "Its file or folder can’t be found. It may have been moved, renamed or deleted, or its disk disconnected."
            }
            Self::ChangedElsewhere => {
                "The file was moved, deleted or changed by something else since you opened it."
            }
            Self::NoSafeReplace => {
                "This disk can’t replace a file safely, so the file was left as it is."
            }
            Self::InProgress => "Another save hadn’t finished.",
            Self::Other => "Something went wrong while writing it.",
        }
    }
}
/// A save that did not happen: said in a sheet, never only in the status
/// bar. Save As writes the open changes to a new file from what the session
/// holds.
pub fn save_failed(name: &str, failure: WriteFailure, file_exists: bool) -> Alert<Action> {
    let mut buttons = vec![Button::new("Save As…", Action::SaveAs).default()];
    if file_exists {
        buttons.push(Button::new("Show in Finder", Action::ShowInFinder));
    }
    buttons.push(Button::new("OK", Action::Ok).cancel());
    Alert::new(
        format!("“{name}” couldn’t be saved."),
        format!(
            "{} Your changes are still open and are kept for recovery. Save As writes them to a new file. Window ▸ Error Log has the details.",
            failure.sentence()
        ),
        buttons,
    )
}
/// An edit that is applied but could not be written to the recovery data:
/// the one moment unsaved work is not protected.
pub fn not_protected(name: &str, failure: WriteFailure) -> Alert<Action> {
    Alert::new(
        format!("Powermove couldn’t protect your latest change to “{name}”."),
        format!(
            "{} The change is in the open project but not in its recovery data, so it would be lost if Powermove quit unexpectedly. Save the project now. Window ▸ Error Log has the details.",
            failure.sentence()
        ),
        vec![
            Button::new("Save", Action::Save).default(),
            Button::new("OK", Action::Ok).cancel(),
        ],
    )
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

#[cfg(test)]
mod write_failure_tests {
    use super::*;

    #[test]
    fn a_failed_write_is_said_in_plain_words_whatever_the_kernel_reported() {
        for (detail, failure) in [
            (
                "/Volumes/Job/cut.pmv: No space left on device (os error 28)",
                WriteFailure::DiskFull,
            ),
            ("Permission denied (os error 13)", WriteFailure::NotAllowed),
            (
                "/x/.cut.pmv.tmp: Read-only file system (os error 30)",
                WriteFailure::ReadOnlyDisk,
            ),
            (
                "The project file was moved, deleted, or changed outside Powermove. Your edits are still open.",
                WriteFailure::ChangedElsewhere,
            ),
            (
                "/x/cut.pmv: No such file or directory (os error 2)",
                WriteFailure::Missing,
            ),
            (
                &pm_kernel::ProjectIoError::NoSafeReplace.to_string(),
                WriteFailure::NoSafeReplace,
            ),
            (
                "A save to this project file is already in progress.",
                WriteFailure::InProgress,
            ),
            ("Storage(\"unexpected\")", WriteFailure::Other),
        ] {
            assert_eq!(WriteFailure::of(detail), failure, "{detail}");
            let alert = save_failed("cut", failure, true);
            assert_eq!(alert.message, "“cut” couldn’t be saved.");
            assert_eq!(alert.labels(), ["Save As…", "Show in Finder", "OK"]);
            assert!(alert.buttons[0].default);
            // Nothing of the diagnostic reaches the sheet.
            for raw in ["os error", "Storage(", "Some(", "Err(", "/x/", "/Volumes/"] {
                assert!(
                    !alert.informative.contains(raw),
                    "{raw} in {}",
                    alert.informative
                );
            }
            assert!(alert.informative.starts_with(failure.sentence()));
            assert!(
                alert
                    .informative
                    .contains("Save As writes them to a new file")
            );
        }
        assert_eq!(
            save_failed("cut", WriteFailure::Missing, false).labels(),
            ["Save As…", "OK"]
        );
        let alert = not_protected("cut", WriteFailure::DiskFull);
        assert_eq!(alert.labels(), ["Save", "OK"]);
        assert!(alert.informative.starts_with("The disk is full."));
        assert!(
            alert
                .informative
                .contains("would be lost if Powermove quit unexpectedly")
        );
    }
}
