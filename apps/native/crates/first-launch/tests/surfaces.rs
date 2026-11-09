use pm_first_launch::{Event, Outcome, Step, surfaces::migration as m};

#[test]
fn n001_raw_progress_pause_resume_and_announcement_are_bounded() {
    let mut model = m::Model::default();
    model.event(&Event::Started(Step::Media));
    assert_eq!(model.progress.as_ref().unwrap().determinate(), None);
    model.event(&Event::Progress {
        step: Step::Media,
        done: 2,
        total: 7,
    });
    assert_eq!(model.progress.as_ref().unwrap().determinate(), Some((2, 7)));
    assert_eq!(model.view().status.as_deref(), Some("Copying media…"));
    // The card carries the numbers: earlier steps read Done, later ones nothing.
    model.bytes = Some((1_500_000, 6_000_000));
    let rows = model.rows();
    let titles: Vec<_> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Library snapshot",
            "Media check",
            "Projects",
            "Media",
            "Settings and extensions"
        ]
    );
    assert_eq!(rows[0].value, "Done");
    assert_eq!(rows[2].value, "Done");
    assert_eq!(rows[3].value, "2 of 7 · 1.5 MB of 6.0 MB");
    // Bytes say more about time left than records do.
    assert_eq!(rows[3].active, Some(Some(0.25)));
    assert_eq!(rows[4].value, "");
    assert_eq!(rows[4].active, None);
    model.pause_at_boundary();
    // Not paused until the worker says so; until then nothing can be resumed.
    assert_eq!(model.view().default, None);
    assert_eq!(model.view().status.as_deref(), Some("Pausing…"));
    model.resume();
    assert_eq!(model.gate, m::Gate::Pausing);
    model.gate = m::Gate::Paused;
    assert_eq!(model.view().default, Some("resume"));
    let frozen = model.progress.clone();
    model.resume();
    assert_eq!(model.progress, frozen);
    assert_eq!(
        model.view().status.as_deref(),
        Some("Picking up where it stopped…")
    );
}
#[test]
fn n001_low_disk_says_how_much_and_a_stranger_asks_for_a_check() {
    let mut model = m::Model {
        gate: m::Gate::WaitingUnknown,
        ..Default::default()
    };
    assert_eq!(model.view().default, Some("check_again"));
    model.gate = m::Gate::LowDisk;
    assert_eq!(
        model.low_disk_view("8 GB", "4 GB").line,
        "Powermove needs 8 GB free to copy your library, and this Mac has 4 GB. Nothing was deleted."
    );
}
#[test]
fn n002_defaults_skip_and_denial_match_spec_without_secret_counts() {
    let mut model = m::Model::default();
    model.outcome(&Outcome::NeedsSecretConsent { interrupted: false });
    let view = model.view();
    assert_eq!(view.line, pm_first_launch::secrets::PROMPT);
    assert_eq!(view.default, Some("resume_with_secret_consent"));
    assert!(
        view.buttons
            .iter()
            .any(|b| b.id == "skip_secret_consent" && b.label == "Not Now")
    );
    model.event(&Event::Started(Step::Secrets));
    assert_eq!(model.view().surface, "N002");
    assert!(model.view().buttons.iter().all(|b| !b.enabled));
    model.gate = m::Gate::Denied;
    assert_eq!(model.view().default, Some("skip_secret_consent"));
    assert!(
        model
            .view()
            .buttons
            .iter()
            .any(|b| b.label == "Continue Without Them")
    );
    model.outcome(&Outcome::NeedsSecretConsent { interrupted: true });
    assert_eq!(model.view().title, "The Keychain Request Didn’t Finish");
}
#[test]
fn n003_summary_omits_zero_rows_and_report_promises_no_way_back() {
    let mut model = m::Model::default();
    model.outcome(&Outcome::Complete {
        projects: 3,
        history_refusals: 1,
        missing_projects: 0,
        blocked_owners: 0,
        proxy_only: 2,
        already_offline: 0,
        not_reached: 0,
    });
    assert_eq!(model.summary.as_ref().unwrap().rows().len(), 2);
    assert_eq!(model.view().default, Some("open_native"));
    assert!(model.view().buttons.iter().any(|b| b.id == "show_report"));
    let mut report = m::Report::default();
    assert_eq!(
        report.empty_copy(),
        Some(("Nothing to Review", "Everything came across."))
    );
    report.rows.push(m::ReportRow {
        id: "kept".into(),
        display_name: "Synthetic".into(),
        kind: m::ReportKind::Conflicts,
    });
    assert_eq!(report.sections().len(), 1);
    assert_eq!(m::ReportKind::Conflicts.section().1, "Show in Finder");
    assert!(
        report
            .footer_actions()
            .iter()
            .any(|button| button.id == "about_native_files")
    );
    assert!(m::about_native_files().line.contains("doesn’t move back"));
    assert_eq!(m::about_native_files().buttons.len(), 1);
    assert_eq!(m::about_native_files().buttons[0].label, "OK");
    assert!(!m::project_open_allowed(true));
    assert!(m::project_open_allowed(false));
    assert_eq!(m::blocked_project_view("Synthetic").default, Some("relink"));
}
#[test]
fn skipped_signins_add_report_action_without_a_secret_count_on_summary() {
    let mut model = m::Model::default();
    model.report(&m::Report {
        rows: vec![m::ReportRow {
            id: "synthetic-account".into(),
            display_name: "Synthetic account".into(),
            kind: m::ReportKind::SignIns,
        }],
    });
    model.outcome(&Outcome::Complete {
        projects: 1,
        history_refusals: 0,
        missing_projects: 0,
        blocked_owners: 0,
        proxy_only: 0,
        already_offline: 0,
        not_reached: 0,
    });
    assert!(
        model
            .view()
            .buttons
            .iter()
            .any(|button| button.id == "show_report")
    );
    assert!(model.summary.as_ref().unwrap().rows().is_empty());
    assert!(!model.view().line.contains("Synthetic account"));
}
#[test]
fn n001_folder_access_is_explained_before_macos_asks_and_waiting_can_be_skipped() {
    let mut model = m::Model::default();
    model.event(&Event::Started(Step::Inventory));
    model.event(&Event::Progress {
        step: Step::Inventory,
        done: 3,
        total: 12,
    });
    assert_eq!(model.rows()[1].value, "3 of 12 projects");
    assert_eq!(model.rows()[1].active, Some(Some(0.25)));
    model.outcome(&Outcome::NeedsFolderAccess {
        folders: vec![("Downloads".into(), 2), ("Desktop".into(), 1)],
    });
    let view = model.view();
    assert_eq!(view.title, "Allow Access to Your Media");
    assert!(
        view.line
            .contains("in Downloads and Desktop, which macOS protects")
    );
    assert_eq!(view.default, Some("allow_folders"));
    let ids: Vec<_> = view.buttons.iter().map(|b| (b.id, b.label)).collect();
    assert_eq!(
        ids,
        [
            ("quit", "Quit Powermove"),
            ("skip_folders", "Skip These Files"),
            ("allow_folders", "Continue")
        ]
    );
    let rows = model.rows();
    assert_eq!(
        (rows[0].title.as_str(), rows[0].value.as_str()),
        ("Downloads", "2 files")
    );
    assert_eq!(
        (rows[1].title.as_str(), rows[1].value.as_str()),
        ("Desktop", "1 file")
    );
    // Continue: copying again, and macOS's prompt is waiting on an answer.
    model.event(&Event::Started(Step::Inventory));
    model.waiting_folder = Some("Downloads".into());
    let view = model.view();
    assert_eq!(view.title, "Waiting for macOS");
    assert!(view.line.contains("open your files in Downloads"));
    // Return never skips; Skip These Files is a deliberate click.
    assert_eq!(view.default, None);
    assert!(view.buttons.iter().any(|b| b.id == "skip_folder"));
    assert!(model.rows().is_empty());
    model.waiting_folder = None;
    assert_eq!(model.view().title, "Copying Your Library");
}
#[test]
fn n003_media_not_reached_is_reported_and_stays_recoverable() {
    let mut model = m::Model::default();
    model.outcome(&Outcome::Complete {
        projects: 2,
        history_refusals: 0,
        missing_projects: 0,
        blocked_owners: 0,
        proxy_only: 0,
        already_offline: 0,
        not_reached: 2,
    });
    let rows = model.summary.as_ref().unwrap().rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "not_reached");
    assert!(rows[0].1.starts_with("2 media files weren’t reached"));
    assert!(rows[0].1.contains("relink"));
    assert!(model.view().buttons.iter().any(|b| b.id == "show_report"));
    assert_eq!(
        m::ReportKind::NotReached.section(),
        ("Media Not Reached", "Relink…")
    );
}
#[test]
fn gate_copy_names_the_app_powermove() {
    let mut model = m::Model::default();
    for gate in [
        m::Gate::WaitingLocal,
        m::Gate::WaitingUnknown,
        m::Gate::Copying,
        m::Gate::Paused,
        m::Gate::LowDisk,
        m::Gate::Failed,
        m::Gate::ProfileRefused,
        m::Gate::Consent,
        m::Gate::InterruptedConsent,
        m::Gate::Denied,
        m::Gate::FolderAccess,
        m::Gate::Complete,
    ] {
        model.gate = gate;
        let view = model.low_disk_view("8 GB", "4 GB");
        let text = format!(
            "{} {} {:?} {:?}",
            view.title, view.line, view.status, view.buttons
        );
        assert!(!text.contains("Native"), "{gate:?}: {text}");
    }
}
