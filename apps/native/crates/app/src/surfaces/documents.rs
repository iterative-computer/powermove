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
    OpenThisOne,
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
        } else if has(&[
            "no such file",
            "os error 2)",
            "not found",
            // A disk that was disconnected while it was written to.
            "bad file descriptor",
            "os error 9)",
            "device not configured",
            "os error 6)",
            "input/output error",
            "os error 5)",
        ]) {
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
/// The kinds of kept file [`kept_beside`] has a sentence for.
const KEPT_KINDS: [&str; 9] = [
    "arrived",
    "other",
    "changed",
    "unfinished",
    "undecided",
    "unconfirmed",
    "earlier",
    "hidden",
    "trashed",
];

/// Which of `rows` [`kept_beside`] tells: the first file of each kind (it
/// has one sentence for each) and every row about no file. The caller marks
/// exactly these seen once the person has answered, and asks again for the
/// rest.
pub fn kept_beside_told(rows: &[(String, Option<String>)]) -> Vec<usize> {
    let mut told: Vec<usize> = KEPT_KINDS
        .iter()
        .filter_map(|kind| {
            rows.iter()
                .position(|(why, name)| why == kind && name.is_some())
        })
        .collect();
    told.extend(
        rows.iter()
            .enumerate()
            .filter(|(_, (why, name))| name.is_none() && (why == "restored" || why == "earlier"))
            .map(|(at, _)| at),
    );
    told.sort_unstable();
    told
}

/// Files found beside a project's file that the person was never told
/// about, said once when the project opens. `rows` are the kernel's
/// (`ShellReply::Kept`): why each file is there and its name. `None` when
/// there is nothing to say.
pub fn kept_beside(file: &str, rows: &[(String, Option<String>)]) -> Option<Alert<Action>> {
    let told = kept_beside_told(rows);
    if told.is_empty() {
        return None;
    }
    let named = |why: &str| {
        told.iter()
            .map(|at| &rows[*at])
            .find(|(w, name)| w == why && name.is_some())
            .and_then(|(_, name)| name.clone())
    };
    let restored = rows.iter().any(|(why, _)| why == "restored");
    let (arrived, other, unfinished) = (named("arrived"), named("other"), named("unfinished"));
    let (undecided, unconfirmed) = (named("undecided"), named("unconfirmed"));
    let (earlier, hidden, trashed) = (named("earlier"), named("hidden"), named("trashed"));
    let changed = named("changed");
    // An earlier save's own account of having put a version back.
    let earlier_restored = rows
        .iter()
        .any(|(why, name)| why == "earlier" && name.is_none());
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
    if let Some(kept) = &changed {
        said.push(format!(
            "An earlier version of “{file}” was changed by another app after Powermove had replaced it, so it was not removed. It is next to “{file}” as “{kept}”. Look at it before you remove it."
        ));
    }
    if let Some(kept) = &earlier {
        said.push(format!(
            "An earlier save kept a file next to “{file}” as “{kept}”. “{file}” has been changed by another app since then, so Powermove can’t tell which version that file goes with. Look at it before you remove it."
        ));
    } else if earlier_restored {
        said.push(format!(
            "An earlier save of “{file}” was interrupted and its last saved version was put back. “{file}” has been changed by another app since then."
        ));
    }
    if let Some(kept) = &hidden {
        said.push(format!(
            "A file from an earlier save of “{file}” was changed by another app and couldn’t be given a visible name. It is still in the project’s folder as the hidden file “{kept}”."
        ));
    }
    if trashed.is_some() {
        said.push(format!(
            "A file from an earlier save of “{file}” was changed by another app and couldn’t be kept next to the project. It was moved to the Trash."
        ));
    }
    said.push("Nothing was lost.".to_owned());
    let kept_another = arrived.is_some() || other.is_some() || changed.is_some();
    let message = if restored || (arrived.is_none() && unfinished.is_some()) || undecided.is_some()
    {
        format!("Powermove was interrupted while saving “{file}”.")
    } else if unconfirmed.is_some() && !kept_another {
        format!("The last save of “{file}” may not have arrived.")
    } else if kept_another {
        format!("Another version of “{file}” was kept.")
    } else {
        format!("An earlier save of “{file}” left a file behind.")
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
/// A version of a project a save's record proves, offered when the file at
/// the project's name does not open (a save cut short, a sync client that
/// changed the file it left): `saving` is the one being saved, otherwise the
/// last saved one. `at` is when that save began, as the person reads it.
pub struct OfferedVersion<'a> {
    pub saving: bool,
    pub at: &'a str,
    pub bytes: u64,
    /// The save that left it was begun on this other computer, which may
    /// still be saving.
    pub computer: Option<&'a str>,
}

/// The file `file` did not open, and a whole version of it exists beside
/// it: which one, from when, how large, with a way to open it.
pub fn version_offer(name: &str, file: &str, version: &OfferedVersion) -> Alert<Action> {
    let which = if version.saving {
        "the version Powermove was saving"
    } else {
        "the last saved version"
    };
    let which_one = if version.saving {
        "The version being saved"
    } else {
        "The last saved version"
    };
    let (at, size) = (version.at, size_text(version.bytes));
    let informative = match version.computer {
        Some(computer) => format!(
            "A save of “{file}” from the computer “{computer}” has not finished. If that computer is still saving, wait for it and open the project again. Otherwise, {} from {at}, {size}, is next to where the project was. Open This One makes it “{file}”; a file under that name now is kept next to it, so nothing is lost.",
            which_one.to_lowercase()
        ),
        None => format!(
            "The file “{file}” is incomplete or was changed after a save was interrupted. {which_one} from {at}, {size}, is next to it. Open This One puts it in place of “{file}”; the file there now is kept next to it, so nothing is lost."
        ),
    };
    Alert::new(
        format!("“{name}” couldn’t be opened, but {which} is intact."),
        informative,
        vec![
            Button::new("Open This One", Action::OpenThisOne).default(),
            Button::new("Show in Finder", Action::ShowInFinder),
            Button::new("Cancel", Action::Cancel).cancel(),
        ],
    )
}

/// The file `file` did not open, and a save of it that did not finish left
/// `count` files beside it that could not be identified. None is offered as
/// a version; the person is told they are there and how to see them.
pub fn unidentified_leftovers(name: &str, file: &str, count: usize) -> Alert<Action> {
    let (files, they_are, them) = if count == 1 {
        ("a file", "It is", "it")
    } else {
        ("files", "They are", "them")
    };
    Alert::new(
        format!("“{name}” couldn’t be opened."),
        format!(
            "The file “{file}” is incomplete or damaged. A save of it that did not finish left {files} next to it that Powermove couldn’t identify, so nothing was put in its place. {they_are} hidden: in Finder, open the project’s folder and press Shift-Command-Period to see {them}. Nothing was moved or removed."
        ),
        vec![
            Button::new("OK", Action::Ok).default(),
            Button::new("Show in Finder", Action::ShowInFinder),
        ],
    )
}

/// `285 MB`, `12 KB`: decimal units, as Finder shows sizes.
fn size_text(bytes: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < units.len() {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else if size < 10.0 {
        format!("{size:.1} {}", units[unit])
    } else {
        format!("{size:.0} {}", units[unit])
    }
}

/// Why a project didn't open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenFailure {
    Service(ServiceErrorCode),
    /// The recovery journal that protects unsaved work couldn't start.
    Journal,
    /// The file is the start of a copy that is still arriving (a sync
    /// client, another computer's save): not damaged, not finished.
    StillCopying,
}
pub fn open_error(name: &str, failure: OpenFailure, path_exists: bool) -> Alert<Action> {
    let (message, informative) = match failure {
        OpenFailure::Service(code) => service_copy(code, name),
        OpenFailure::Journal => (
            format!("“{name}” couldn’t be opened."),
            "Powermove couldn’t start the recovery journal that protects unsaved work. Check free space and permissions in your library, then try again.",
        ),
        OpenFailure::StillCopying => (
            format!("“{name}” is still being copied to this Mac."),
            "Wait until the copy or sync is done, then open it again. Nothing was changed.",
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
    fn a_version_offered_after_a_failed_open_says_which_when_and_how_large() {
        let alert = version_offer(
            "cut",
            "cut.pmv",
            &OfferedVersion {
                saving: true,
                at: "2026-11-12 14.03.22",
                bytes: 285_283_213,
                computer: None,
            },
        );
        assert_eq!(
            alert.message,
            "“cut” couldn’t be opened, but the version Powermove was saving is intact."
        );
        assert!(
            alert
                .informative
                .contains("from 2026-11-12 14.03.22, 285 MB")
        );
        assert_eq!(
            alert.labels(),
            ["Open This One", "Show in Finder", "Cancel"]
        );
        assert!(alert.buttons[0].default);
        assert!(!alert.informative.contains('/'), "names, never paths");
        assert_eq!(size_text(12_400), "12 KB");
        assert_eq!(size_text(1_500_000_000), "1.5 GB");
    }

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

    /// A kept file whose project was changed by another app since, a file
    /// that could not be given a visible name and one that went to the
    /// Trash are each said in plain words, with no path and no word of the
    /// app's own workings.
    #[test]
    fn files_an_earlier_save_left_are_said_without_claiming_whose_they_are() {
        let row = |why: &str, name: Option<&str>| (why.to_owned(), name.map(str::to_owned));
        let alert = kept_beside(
            "cut.pmv",
            &[row(
                "earlier",
                Some("cut (other version 2026-11-12 14.03.22).pmv"),
            )],
        )
        .unwrap();
        assert_eq!(
            alert.message,
            "An earlier save of “cut.pmv” left a file behind."
        );
        assert!(
            alert
                .informative
                .contains("as “cut (other version 2026-11-12 14.03.22).pmv”")
        );
        assert!(alert.informative.contains("changed by another app since"));
        let alert = kept_beside(
            "cut.pmv",
            &[
                row("hidden", Some(".cut.pmv.0f8f.gone")),
                row("trashed", Some(".cut.pmv.0f8f.aside")),
            ],
        )
        .unwrap();
        assert!(
            alert
                .informative
                .contains("the hidden file “.cut.pmv.0f8f.gone”")
        );
        assert!(alert.informative.contains("moved to the Trash"));
        assert!(
            !alert.informative.contains(".cut.pmv.0f8f.aside"),
            "a file in the Trash is not named by where it was"
        );
        assert!(!alert.informative.contains('/'), "names, never paths");
        assert!(alert.informative.ends_with("Nothing was lost."));
    }

    /// A version that was to be removed and had been written into is said
    /// as that, not as a save that was interrupted.
    #[test]
    fn an_earlier_version_found_changed_is_said_as_changed() {
        let alert = kept_beside(
            "cut.pmv",
            &[(
                "changed".to_owned(),
                Some("cut (other version 2026-11-12 14.03.22).pmv".to_owned()),
            )],
        )
        .unwrap();
        assert_eq!(alert.message, "Another version of “cut.pmv” was kept.");
        assert!(alert.informative.contains("was changed by another app"));
        assert!(!alert.informative.contains("interrupted"));
        assert!(alert.informative.ends_with("Nothing was lost."));
    }

    /// One sheet tells the first file of each kind; the rows it tells are
    /// the only ones the caller may mark seen.
    #[test]
    fn the_rows_a_sheet_tells_are_exactly_the_ones_it_names() {
        let row = |why: &str, name: Option<&str>| (why.to_owned(), name.map(str::to_owned));
        let rows = [
            row("arrived", Some("cut (other version 1).pmv")),
            row("arrived", Some("cut (other version 2).pmv")),
            row("restored", None),
            row("unknown kind", Some("x")),
        ];
        assert_eq!(kept_beside_told(&rows), [0, 2]);
        let alert = kept_beside("cut.pmv", &rows).unwrap();
        assert!(alert.informative.contains("cut (other version 1).pmv"));
        assert!(!alert.informative.contains("cut (other version 2).pmv"));
        assert!(kept_beside_told(&[row("unknown kind", Some("x"))]).is_empty());
    }

    /// Leftovers nobody could identify are said to be there, never offered.
    #[test]
    fn files_a_save_left_that_could_not_be_identified_are_said_not_offered() {
        let alert = unidentified_leftovers("cut", "cut.pmv", 2);
        assert_eq!(alert.message, "“cut” couldn’t be opened.");
        assert!(alert.informative.contains("left files next to it"));
        assert!(alert.informative.contains("Shift-Command-Period"));
        assert!(alert.informative.ends_with("Nothing was moved or removed."));
        assert_eq!(alert.labels(), ["OK", "Show in Finder"]);
        assert!(
            unidentified_leftovers("cut", "cut.pmv", 1)
                .informative
                .contains("left a file next to it")
        );
    }

    /// A disk that went away while it was written to is said as that.
    #[test]
    fn a_disk_that_went_away_mid_save_is_said_as_a_missing_place() {
        for detail in [
            "/Volumes/Job/.cut.pmv.0f8f.save: Bad file descriptor (os error 9)",
            "/Volumes/Job/cut.pmv: Device not configured (os error 6)",
            "/Volumes/Job/cut.pmv: Input/output error (os error 5)",
        ] {
            assert_eq!(WriteFailure::of(detail), WriteFailure::Missing, "{detail}");
        }
    }

    /// A save another computer began is offered with that said: it may
    /// still be saving.
    #[test]
    fn a_version_another_computer_left_is_offered_with_that_said() {
        let alert = version_offer(
            "cut",
            "cut.pmv",
            &OfferedVersion {
                saving: false,
                at: "2026-11-12 14.03.22",
                bytes: 285_283_213,
                computer: Some("studio-mac-2"),
            },
        );
        assert_eq!(
            alert.message,
            "“cut” couldn’t be opened, but the last saved version is intact."
        );
        assert!(alert.informative.contains("the computer “studio-mac-2”"));
        assert!(
            alert
                .informative
                .contains("If that computer is still saving")
        );
        assert_eq!(
            alert.labels(),
            ["Open This One", "Show in Finder", "Cancel"]
        );
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
