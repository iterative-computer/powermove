//! N004–N006: the extension step's copy, counts, lists and buttons, at startup
//! and from Settings › Storage, on synthetic scans and results.
use pm_first_launch::{
    extension_transfer::{
        ExtensionResult, Progress, Scan, ScanEntry, ScanStatus, TransferOutcome, TransferResult,
    },
    surfaces::extensions::{Gate, Model, Phase, Place, Tone},
};

fn entry(id: &str, status: ScanStatus, reason: &str) -> ScanEntry {
    ScanEntry {
        id: id.into(),
        status,
        reason: reason.into(),
    }
}
/// The shape of Chike's original app: 44 new, 1 already here, 1 empty folder.
fn scan() -> Scan {
    let mut entries: Vec<_> = (0..44)
        .map(|i| {
            entry(
                &format!("ext-{i:02}"),
                ScanStatus::New,
                "New in the original app.",
            )
        })
        .collect();
    entries.push(entry(
        "youtube-import",
        ScanStatus::AlreadyPresent,
        "Already in Powermove.",
    ));
    entries.push(entry(
        "figma-layer-import",
        ScanStatus::Skipped,
        "Its folder is empty, so there’s nothing to bring in.",
    ));
    Scan {
        found: 45,
        new: 44,
        changed: 1,
        already_present: 1,
        entries,
        ..Scan::default()
    }
}
fn ids(model: &Model) -> Vec<&'static str> {
    model.view().buttons.iter().map(|b| b.id).collect()
}

#[test]
fn n004_names_how_many_were_found_and_what_is_skipped_and_why() {
    let model = Model::offer(scan(), Place::Startup);
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
    let rows: Vec<_> = model
        .rows()
        .into_iter()
        .map(|r| (r.title, r.value))
        .collect();
    assert_eq!(
        rows,
        [
            ("New".into(), "44".into()),
            ("Updated in the original app".into(), "1".into()),
            ("Already in Powermove".into(), "1".into()),
            ("Skipped".into(), "1".into()),
        ]
    );
    let items = model.items();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].text,
        "figma-layer-import: Its folder is empty, so there’s nothing to bring in."
    );
    assert_eq!(items[0].tone, Tone::Note);
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
fn n005_shows_the_phase_and_how_far_it_has_got() {
    let mut model = Model::offer(scan(), Place::Startup);
    model.start();
    assert_eq!(model.gate, Gate::Running);
    assert_eq!(model.view().status.as_deref(), Some("Getting ready…"));
    assert_eq!(
        model.rows()[0].active,
        Some(None),
        "no total yet: indeterminate"
    );
    assert!(model.view().buttons.is_empty());
    // The engine copies all 46, then checks each: done runs over 92.
    model.progress(Progress {
        done: 11,
        total: 92,
        current: "ext-11".into(),
    });
    assert_eq!(model.phase(), Some((Phase::Copying, 12, 46)));
    assert_eq!(model.view().status.as_deref(), Some("Copying ext-11…"));
    let row = &model.rows()[0];
    assert_eq!(
        (row.title.as_str(), row.value.as_str()),
        ("Copying", "12 of 46")
    );
    model.progress(Progress {
        done: 60,
        total: 92,
        current: "ext-14".into(),
    });
    assert_eq!(model.phase(), Some((Phase::Checking, 15, 46)));
    assert_eq!(
        model.view().status.as_deref(),
        Some("Checking that ext-14 loads…")
    );
    assert_eq!(model.rows()[0].active, Some(Some(60. / 92.)));
}
#[test]
fn n006_summarises_every_extension_with_its_reason_problems_first() {
    let mut model = Model::offer(scan(), Place::Startup);
    model.start();
    let result = |id: &str, outcome, reason: &str| ExtensionResult {
        id: id.into(),
        outcome,
        reason: reason.into(),
    };
    model.finished(Ok(TransferResult {
        brought_in: 43,
        already_present: 1,
        could_not_load: 1,
        entries: vec![
            result(
                "anchor-point",
                TransferOutcome::BroughtIn,
                "New in the original app. It loads.",
            ),
            result(
                "displacement-origin-guide",
                TransferOutcome::CouldNotLoad,
                "It was copied but couldn’t load: requires radial-directional-displacement",
            ),
            result(
                "figma-layer-import",
                TransferOutcome::Skipped,
                "Its folder is empty, so there’s nothing to bring in.",
            ),
            result(
                "youtube-import",
                TransferOutcome::AlreadyPresent,
                "Already in Powermove.",
            ),
        ],
    }));
    let view = model.view();
    assert_eq!(view.surface, "N006");
    assert_eq!(view.title, "Your Extensions Are In");
    assert_eq!(
        view.line,
        "43 brought in, 1 already in Powermove and 1 couldn’t load. Your original app stays unchanged."
    );
    assert_eq!(ids(&model), ["open_native"]);
    // The line gives the counts; the summary's room goes to the list.
    assert!(model.rows().is_empty());
    let items = model.items();
    let order: Vec<_> = items.iter().map(|i| (i.id.as_str(), i.tone)).collect();
    assert_eq!(
        order,
        [
            ("displacement-origin-guide", Tone::Warning),
            ("figma-layer-import", Tone::Note),
            ("youtube-import", Tone::Note),
            ("anchor-point", Tone::Done),
        ]
    );
    assert_eq!(
        items[0].text,
        "displacement-origin-guide: It was copied but couldn’t load: requires radial-directional-displacement"
    );
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
    assert!(model.rows().is_empty() && model.items().is_empty());
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
        "46 already in Powermove. Your original app stays unchanged."
    );
}
