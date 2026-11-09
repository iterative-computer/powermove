use super::*;

fn on_return(a: &Alert<Action>) -> Option<Action> {
    a.buttons.iter().find(|b| b.default).map(|b| b.action)
}
fn on_key(a: &Alert<Action>, key: Key) -> Option<Action> {
    a.buttons.iter().find(|b| b.key == key).map(|b| b.action)
}
#[test]
fn close_copy_and_keyboard_are_exact() {
    let a = unsaved("Launch Film");
    assert_eq!(a.message, "Save changes to “Launch Film” before closing?");
    assert_eq!(
        a.informative,
        "Your project file hasn’t been updated. Local recovery is kept separately."
    );
    assert_eq!(a.labels(), ["Save", "Cancel", "Don’t Save"]);
    assert_eq!(on_return(&a), Some(Action::Save));
    assert_eq!(on_key(&a, Key::Escape), Some(Action::Cancel));
    assert_eq!(on_key(&a, Key::CommandD), Some(Action::DontSave));
}
#[test]
fn multi_quit_copy_and_default_are_exact() {
    let a = review_quit(3);
    assert_eq!(
        a.message,
        "You have 3 projects with unsaved changes. Do you want to review these changes before quitting?"
    );
    assert_eq!(
        a.informative,
        "If you discard them, their project files aren’t updated. Local recovery is kept separately."
    );
    assert_eq!(a.labels(), ["Review Changes…", "Cancel", "Discard Changes"]);
    assert_eq!(on_return(&a), Some(Action::ReviewChanges));
    assert_eq!(on_key(&a, Key::Escape), Some(Action::Cancel));
    assert_eq!(a.buttons[2].key, Key::None);
}
#[test]
fn service_copy_table_is_exact() {
    use ServiceErrorCode::*;
    let generic = (
        "The operation couldn’t be completed.",
        "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
    );
    for (code, (message, info)) in [
        (
            Offline,
            (
                "Powermove can’t connect.",
                "Check your internet connection and try again.",
            ),
        ),
        (
            NotFound,
            (
                "“Film” can’t be found.",
                "It may have been moved or deleted.",
            ),
        ),
        (TooLarge, ("“Film” is too large to open.", "")),
        (
            InvalidProject,
            (
                "“Film” can’t be opened because it isn’t a Powermove project.",
                "",
            ),
        ),
        (
            PermissionDenied,
            (
                "You don’t have permission to open “Film”.",
                "Check its sharing and permissions in Finder.",
            ),
        ),
        (
            DiskFull,
            (
                "There isn’t enough disk space.",
                "Free some space and try again.",
            ),
        ),
        (
            InvalidInput,
            ("“Film” couldn’t be read.", "The file may be damaged."),
        ),
        (
            InvalidData,
            ("“Film” couldn’t be read.", "The file may be damaged."),
        ),
        (Unavailable, generic),
        (Unknown, generic),
    ] {
        assert_eq!(service_copy(code, "Film"), (message.into(), info));
    }
}
#[test]
fn open_errors_only_offer_finder_for_existing_paths_and_ok_is_default() {
    for exists in [true, false] {
        let a = open_error(
            "Film",
            OpenFailure::Service(ServiceErrorCode::InvalidData),
            exists,
        );
        assert_eq!(on_return(&a), Some(Action::Ok));
        assert_eq!(
            a.buttons.iter().any(|b| b.action == Action::ShowInFinder),
            exists
        );
        assert_eq!(a.buttons[0].label, "OK");
    }
    let a = open_error("Film", OpenFailure::Journal, false);
    assert_eq!(a.message, "“Film” couldn’t be opened.");
    assert!(a.informative.contains("recovery journal"));
}
