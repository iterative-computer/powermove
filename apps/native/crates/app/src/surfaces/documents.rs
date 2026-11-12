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
/// A save that went through while another version of the file arrived (a
/// sync app, another computer): both are on disk, and the person is told
/// which is which and where the other one is.
pub fn other_version_kept(name: &str, file: &str, kept: &str) -> Alert<Action> {
    Alert::new(
        format!("“{name}” was saved, and another version of its file was kept."),
        format!(
            "While Powermove was saving, something else (a sync app or another computer) put a different version of “{file}” in its place. Your save is in “{file}”. The other version is next to it as “{kept}”. Nothing was lost; open the other version to see what it holds."
        ),
        vec![
            Button::new("Show in Finder", Action::ShowInFinder).default(),
            Button::new("OK", Action::Ok).cancel(),
        ],
    )
}
/// A save that could not take its file's name: another version landed there
/// while the file was being replaced, and was left in place. The save's own
/// file is next to it, and the changes are still open.
pub fn saved_beside(name: &str, file: &str, kept: &str) -> Alert<Action> {
    Alert::new(
        format!("“{name}” was saved next to its file, not over it."),
        format!(
            "While Powermove was saving, something else (a sync app or another computer) put a different version of “{file}” in its place. That version was left where it is. Your save is next to it as “{kept}”. Your changes are also still open: Save As writes them to a new file."
        ),
        vec![
            Button::new("Save As…", Action::SaveAs).default(),
            Button::new("Show in Finder", Action::ShowInFinder),
            Button::new("OK", Action::Ok).cancel(),
        ],
    )
}
/// Files found beside a project's file that the person was never told
/// about, said once when the project opens. `rows` are the kernel's
/// (`ShellReply::Kept`): why each file is there and its name. `None` when
/// there is nothing to say.
pub fn kept_beside(file: &str, rows: &[(String, Option<String>)]) -> Option<Alert<Action>> {
    let named = |why: &str| {
        rows.iter()
            .find(|(w, name)| w == why && name.is_some())
            .and_then(|(_, name)| name.clone())
    };
    let restored = rows.iter().any(|(why, _)| why == "restored");
    let (arrived, other, unfinished) = (named("arrived"), named("other"), named("unfinished"));
    let (undecided, unconfirmed) = (named("undecided"), named("unconfirmed"));
    if !restored
        && [&arrived, &other, &unfinished, &undecided, &unconfirmed]
            .iter()
            .all(|kept| kept.is_none())
    {
        return None;
    }
    let mut said = Vec::new();
    if restored {
        said.push("The last saved version is back in place and is the one open now.".to_owned());
    }
    if let Some(kept) = &other {
        said.push(format!(
            "A save of this project was interrupted before it finished. The file it was replacing is next to “{file}” as “{kept}”."
        ));
    }
    if let Some(kept) = &unfinished {
        said.push(format!(
            "The save that was interrupted is next to it as “{kept}”."
        ));
    }
    if let Some(kept) = &arrived {
        said.push(format!(
            "The last time this project was saved, something else (a sync app or another computer) had put a different version of “{file}” in its place. Your save is in “{file}”. The other version is next to it as “{kept}”."
        ));
    }
    if let Some(kept) = &undecided {
        said.push(format!(
            "A save of this project did not finish and left a file Powermove could not identify. It was not moved: it is next to “{file}” as “{kept}”."
        ));
    }
    if unconfirmed.is_some() {
        said.push(format!(
            "The last time “{file}” was saved, its drive could not confirm that the save arrived, and the file there now is not that version. The version you saved is kept on this Mac."
        ));
    }
    said.push("Nothing was lost.".to_owned());
    let kept_another = arrived.is_some() || other.is_some();
    let message = if restored || (arrived.is_none() && unfinished.is_some()) || undecided.is_some()
    {
        format!("Powermove was interrupted while saving “{file}”.")
    } else if unconfirmed.is_some() && !kept_another {
        format!("The last save of “{file}” may not have arrived.")
    } else {
        format!("Another version of “{file}” was kept.")
    };
    Some(Alert::new(
        message,
        said.join(" "),
        vec![
            Button::new("Show in Finder", Action::ShowInFinder).default(),
            Button::new("OK", Action::Ok).cancel(),
        ],
    ))
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
    fn a_version_kept_beside_a_save_is_named_with_a_way_to_it() {
        let alert = other_version_kept(
            "cut",
            "cut.pmv",
            "cut (other version 2026-11-12 14.03.22).pmv",
        );
        assert_eq!(
            alert.message,
            "“cut” was saved, and another version of its file was kept."
        );
        assert!(alert.informative.contains("Your save is in “cut.pmv”."));
        assert!(
            alert
                .informative
                .contains("next to it as “cut (other version 2026-11-12 14.03.22).pmv”")
        );
        assert_eq!(alert.labels(), ["Show in Finder", "OK"]);
        assert!(alert.buttons[0].default);
        assert!(!alert.informative.contains('/'), "names, never paths");
    }

    #[test]
    fn a_save_kept_beside_another_version_says_where_it_is() {
        let detail = pm_kernel::ProjectIoError::KeptBeside {
            kept: "/Volumes/Job/cut (recovered 2026-11-12 14.03.22).pmv".into(),
        }
        .to_string();
        let kept = pm_kernel::project_io::kept_beside(&detail).expect("the failure names it");
        let kept = kept.file_name().unwrap().to_string_lossy();
        let alert = saved_beside("cut", "cut.pmv", &kept);
        assert_eq!(
            alert.message,
            "“cut” was saved next to its file, not over it."
        );
        assert!(
            alert
                .informative
                .contains("next to it as “cut (recovered 2026-11-12 14.03.22).pmv”")
        );
        assert_eq!(alert.labels(), ["Save As…", "Show in Finder", "OK"]);
        assert!(!alert.informative.contains('/'), "names, never paths");
    }

    #[test]
    fn a_save_the_drive_could_not_confirm_and_a_file_left_unmoved_are_said() {
        let alert = kept_beside(
            "cut.pmv",
            &[("unconfirmed".into(), Some("0f8f.pmv".into()))],
        )
        .expect("said");
        assert_eq!(
            alert.message,
            "The last save of “cut.pmv” may not have arrived."
        );
        assert!(alert.informative.contains("kept on this Mac"));
        assert!(alert.informative.ends_with("Nothing was lost."));
        let alert = kept_beside(
            "cut.pmv",
            &[("undecided".into(), Some(".cut.pmv.0f8f.aside".into()))],
        )
        .expect("said");
        assert!(alert.message.contains("interrupted while saving"));
        assert!(alert.informative.contains("It was not moved"));
        assert!(alert.informative.contains("“.cut.pmv.0f8f.aside”"));
    }

    #[test]
    fn files_kept_beside_a_project_are_said_once_it_opens() {
        let row = |why: &str, name: Option<&str>| (why.to_owned(), name.map(str::to_owned));
        assert!(kept_beside("cut.pmv", &[]).is_none());
        // A save at quit kept a version that had arrived.
        let alert = kept_beside(
            "cut.pmv",
            &[row(
                "arrived",
                Some("cut (other version 2026-11-12 14.03.22).pmv"),
            )],
        )
        .unwrap();
        assert_eq!(alert.message, "Another version of “cut.pmv” was kept.");
        assert!(alert.informative.contains("Your save is in “cut.pmv”."));
        assert!(alert.informative.ends_with("Nothing was lost."));
        assert_eq!(alert.labels(), ["Show in Finder", "OK"]);
        // A save was cut short between moving the file aside and installing.
        let alert = kept_beside(
            "cut.pmv",
            &[
                row("restored", None),
                row(
                    "unfinished",
                    Some("cut (recovered 2026-11-12 14.03.22).pmv"),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            alert.message,
            "Powermove was interrupted while saving “cut.pmv”."
        );
        assert_eq!(
            alert.informative,
            "The last saved version is back in place and is the one open now. The save that was interrupted is next to it as “cut (recovered 2026-11-12 14.03.22).pmv”. Nothing was lost."
        );
        // Cut short after installing: the replaced file was still aside.
        let alert = kept_beside(
            "cut.pmv",
            &[row(
                "other",
                Some("cut (other version 2026-11-12 14.03.22).pmv"),
            )],
        )
        .unwrap();
        assert_eq!(alert.message, "Another version of “cut.pmv” was kept.");
        assert!(
            alert
                .informative
                .contains("The file it was replacing is next to “cut.pmv”")
        );
    }

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
