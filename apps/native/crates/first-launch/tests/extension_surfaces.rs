//! N004–N006: the extension step's copy, counts, lists and buttons, at startup
//! and from Settings › Storage, on synthetic scans and results.
use pm_first_launch::{
    extension_transfer::{
        ExtensionResult, Progress, Scan, ScanEntry, ScanStatus, TransferOutcome, TransferResult,
    },
    surfaces::extensions::{Gate, Model, Phase, Place, Tone},
};
use std::collections::BTreeMap;

fn entry(id: &str, status: ScanStatus, reason: &str) -> ScanEntry {
    ScanEntry {
        id: id.into(),
        status,
        reason: reason.into(),
    }
}
/// The shape of Chike's original app: 45 new, 1 empty folder.
fn scan() -> Scan {
    let mut entries: Vec<_> = (0..45)
        .map(|i| {
            entry(
                &format!("ext-{i:02}"),
                ScanStatus::New,
                "New in the original app.",
            )
        })
        .collect();
    entries.push(entry(
        "figma-layer-import",
        ScanStatus::Skipped,
        "Its folder is empty or incomplete, so there’s nothing to bring in.",
    ));
    Scan {
        found: 45,
        new: 45,
        entries,
        ..Scan::default()
    }
}
fn names() -> BTreeMap<String, String> {
    (0..45)
        .map(|i| (format!("ext-{i:02}"), format!("Extension {:02}", 44 - i)))
        .collect()
}
fn ids(model: &Model) -> Vec<&'static str> {
    model.view().buttons.iter().map(|b| b.id).collect()
}
fn result(id: &str, outcome: TransferOutcome, reason: &str) -> ExtensionResult {
    ExtensionResult {
        id: id.into(),
        outcome,
        reason: reason.into(),
    }
}

#[test]
fn n004_leads_with_the_count_shows_skips_in_full_and_lists_the_rest_by_name() {
    let model = Model::offer(scan(), Place::Startup).named(names());
    let view = model.view();
    assert_eq!(view.surface, "N004");
    assert_eq!(view.title, "Bring In Your Extensions");
    assert!(
        view.line
            .starts_with("The original app has 45 extensions that aren’t in Powermove yet."),
        "{}",
        view.line
    );
    assert!(
        view.line
            .contains("projects, settings and sign-ins stay as they are")
    );
    assert_eq!(model.stats(), [(45, "New"), (1, "Skipped")]);
    // The exception, in full: its name and its reason on their own lines.
    let items = model.items();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].name, "figma-layer-import",
        "no manifest: the folder's name"
    );
    assert!(items[0].reason.starts_with("Its folder is empty"));
    assert_eq!(items[0].tone, Tone::Note);
    assert_eq!(model.items_title(), Some("Not Brought In"));
    // The rest by display name only, sorted as read; no reason repeated.
    let (title, listed) = model.listed().unwrap();
    assert_eq!(title, "To Bring In");
    assert_eq!(listed.len(), 45);
    assert_eq!(listed[0], "Extension 00");
    assert!(listed.iter().all(|n| !n.contains("New in")));
    assert!(model.rows().is_empty(), "no counts card: the stats lead");
    // Startup: Not Now never blocks, and says where to find this again.
    assert_eq!(ids(&model), ["not_now", "bring_in"]);
    assert_eq!(view.buttons[1].label, "Bring In Extensions");
    assert_eq!(view.buttons[0].label, "Not Now");
    assert_eq!(view.default, Some("bring_in"));
    assert!(model.note().unwrap().contains("Settings › Storage"));
}
#[test]
fn n004_from_settings_cancels_and_has_no_startup_note() {
    let model = Model::offer(scan(), Place::Settings);
    assert_eq!(ids(&model), ["cancel", "bring_in"]);
    assert_eq!(model.note(), None);
    // Nothing new: the button is there but can't start an empty run.
    let empty = Model::offer(Scan::default(), Place::Settings);
    assert!(!empty.view().buttons[1].enabled);
}
#[test]
fn n005_names_the_extension_and_the_phase_on_one_bar() {
    let mut model = Model::offer(scan(), Place::Startup).named(names());
    model.start();
    assert_eq!(model.gate, Gate::Running);
    assert_eq!(
        model.rows()[0].active,
        Some(None),
        "no total yet: indeterminate"
    );
    assert!(model.view().buttons.is_empty());
    assert!(model.stats().is_empty() && model.listed().is_none());
    // The engine copies all 46, then checks each: done runs over 92.
    model.progress(Progress {
        done: 11,
        total: 92,
        current: "ext-11".into(),
    });
    assert_eq!(model.phase(), Some((Phase::Copying, 12, 46)));
    let row = &model.rows()[0];
    assert_eq!(
        (row.title.as_str(), row.value.as_str()),
        ("Copying", "Extension 33")
    );
    model.progress(Progress {
        done: 60,
        total: 92,
        current: "ext-14".into(),
    });
    assert_eq!(model.phase(), Some((Phase::Checking, 15, 46)));
    let row = &model.rows()[0];
    assert_eq!(
        (row.title.as_str(), row.value.as_str()),
        ("Checking that it loads", "Extension 30")
    );
    assert_eq!(row.active, Some(Some(60. / 92.)));
}
#[test]
fn n006_leads_with_the_outcome_exceptions_first_and_successes_by_name() {
    let mut model = Model::offer(scan(), Place::Startup).named(names());
    model.start();
    let mut entries = vec![
        result(
            "displacement-origin-guide",
            TransferOutcome::CouldNotLoad,
            "It was copied but couldn’t load: requires radial-directional-displacement",
        ),
        result(
            "figma-layer-import",
            TransferOutcome::Skipped,
            "Its folder is empty or incomplete, so there’s nothing to bring in.",
        ),
        result(
            "youtube-import",
            TransferOutcome::AlreadyPresent,
            "Already in Powermove.",
        ),
    ];
    entries.extend((0..43).map(|i| {
        result(
            &format!("ext-{i:02}"),
            TransferOutcome::BroughtIn,
            "New in the original app. It loads.",
        )
    }));
    model.finished(Ok(TransferResult {
        brought_in: 43,
        already_present: 1,
        could_not_load: 1,
        entries,
    }));
    let view = model.view();
    assert_eq!(view.surface, "N006");
    assert_eq!(view.title, "Your Extensions Are In");
    assert_eq!(
        view.line,
        "One couldn’t load. The reason is below. Your original app stays unchanged."
    );
    assert_eq!(
        model.stats(),
        [
            (43, "Brought in"),
            (1, "Couldn’t load"),
            (1, "Already in Powermove"),
            (1, "Skipped")
        ]
    );
    assert_eq!(ids(&model), ["open_native"]);
    assert_eq!(model.items_title(), Some("Needs a Look"));
    let order: Vec<_> = model.items().into_iter().map(|i| (i.id, i.tone)).collect();
    assert_eq!(
        order,
        [
            ("displacement-origin-guide".to_owned(), Tone::Warning),
            ("figma-layer-import".to_owned(), Tone::Note),
            ("youtube-import".to_owned(), Tone::Note),
        ]
    );
    let (title, listed) = model.listed().unwrap();
    assert_eq!(title, "Brought In");
    assert_eq!(listed.len(), 43);
    assert!(!listed.iter().any(|n| n.contains("It loads")));
    // From Settings the summary ends where it started.
    model.place = Place::Settings;
    assert_eq!(ids(&model), ["done"]);
}
#[test]
fn n006_a_stopped_transfer_says_nothing_changed_and_still_opens_the_library() {
    let mut model = Model::offer(scan(), Place::Startup);
    model.start();
    model.finished(Err(
        "Cannot read the original extension folder: permission denied".into(),
    ));
    let view = model.view();
    assert_eq!(model.gate, Gate::Failed);
    assert_eq!(view.title, "Your Extensions Couldn’t Be Brought In");
    assert!(view.line.starts_with("Nothing in your library changed."));
    assert_eq!(ids(&model), ["try_again", "open_native"]);
    assert_eq!(view.default, Some("open_native"));
    assert!(model.details().unwrap().contains("permission denied"));
    assert!(model.rows().is_empty() && model.items().is_empty() && model.stats().is_empty());
    // Progress after the end is ignored.
    model.progress(Progress {
        done: 1,
        total: 2,
        current: "x".into(),
    });
    assert!(model.progress.is_none());
}
#[test]
fn nothing_new_reads_as_such_and_singular_counts_read_right() {
    let mut one = Scan {
        new: 1,
        ..Scan::default()
    };
    one.entries
        .push(entry("pong", ScanStatus::New, "New in the original app."));
    let model = Model::offer(one, Place::Startup);
    assert!(
        model
            .view()
            .line
            .starts_with("The original app has 1 extension that isn’t in Powermove yet.")
    );
    let mut model = Model::offer(scan(), Place::Settings);
    model.start();
    model.finished(Ok(TransferResult {
        already_present: 46,
        ..TransferResult::default()
    }));
    assert_eq!(model.view().title, "Nothing New to Bring In");
    assert_eq!(
        model.view().line,
        "They were already in Powermove. Your original app stays unchanged."
    );
    assert_eq!(model.stats(), [(46, "Already in Powermove")]);
    assert!(model.listed().is_none());
}
