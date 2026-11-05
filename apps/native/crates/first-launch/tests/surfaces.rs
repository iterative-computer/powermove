use pm_first_launch::{
    Event, Outcome, Step,
    surfaces::{migration as m, native as n, updates as u},
};
use serde_json::json;

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
    assert_eq!(
        model.view().status.as_deref(),
        Some("Copying media · 2 of 7")
    );
    model.pause_at_boundary();
    assert_eq!(model.view().default, Some("resume"));
    let frozen = model.progress.clone();
    model.resume();
    assert_eq!(model.progress, frozen);
    assert_eq!(
        model.view().status.as_deref(),
        Some("Picking up where it stopped…")
    );
    assert!(model.announce_progress(0));
    assert!(!model.announce_progress(4999));
    assert!(model.announce_progress(5000));
}
#[test]
fn n001_later_delta_waits_silently_and_never_opens_gate() {
    let mut model = m::Model::default();
    model.later_launch = true;
    assert!(!model.gate_visible());
    assert_eq!(model.background_task(), Some("Bringing in new media"));
    model.event(&Event::WaitingForElectron);
    assert_eq!(model.background_task(), None);
    model.gate = m::Gate::WaitingUnknown;
    assert_eq!(model.view().default, Some("check_again"));
    model.gate = m::Gate::LowDisk;
    assert_eq!(
        model.low_disk_view("8 GB", "4 GB").line,
        "Powermove Native needs 8 GB free to copy your library, and this Mac has 4 GB. Nothing was deleted."
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
            .all(|button| !button.label.contains("Back") && !button.label.contains("Restore"))
    );
    assert!(!m::project_open_allowed(true));
    assert!(m::project_open_allowed(false));
    assert_eq!(m::blocked_project_view("Synthetic").default, Some("relink"));
}
#[test]
fn automatic_update_checks_are_silent_and_ready_survives_cancelled_quit() {
    let mut model = u::Model::new("1.0".into());
    for state in [
        u::State::Checking,
        u::State::UpToDate,
        u::State::Available,
        u::State::Downloading,
        u::State::Verifying,
        u::State::CheckFailed,
        u::State::Offline,
        u::State::DownloadFailed,
    ] {
        model.state = state;
        assert_eq!(model.view(), None);
    }
    model.offered = "2.0".into();
    model.state = u::State::Ready;
    assert!(model.viewer_banner());
    model.dismiss_banner();
    assert!(!model.viewer_banner());
    assert!(model.library_notice());
    assert!(!model.restart_after_barrier(true, true));
    assert!(!model.restart_after_barrier(false, false));
    assert_eq!(model.state, u::State::Ready);
    assert!(model.restart_after_barrier(false, true));
    let saved = serde_json::to_value(&model.preferences).unwrap();
    let mut next = u::Model::new("1.0".into());
    next.preferences = serde_json::from_value(saved).unwrap();
    next.state = u::State::Ready;
    next.offered = "2.0".into();
    assert!(!next.viewer_banner());
    assert!(next.library_notice());
}
#[test]
fn manual_update_failure_and_verification_states_are_explicit() {
    let mut model = u::Model::new("1.0".into());
    model.manual = true;
    model.state = u::State::Downloading;
    model.done = 10;
    model.total = Some(0);
    assert_eq!(model.progress(), None);
    assert!(model.cancel());
    model.state = u::State::Verifying;
    assert!(!model.cancel());
    assert!(model.view().unwrap().buttons.is_empty());
    model.state = u::State::VerificationFailed;
    assert!(model.view().unwrap().title.contains("wasn’t installed"));
    model.state = u::State::Offline;
    assert_eq!(
        model.view().unwrap().line,
        "Check your internet connection, then try again."
    );
    model.state = u::State::Available;
    model.offered = "2.0".into();
    model.skip_version();
    assert!(model.skipped("2.0"));
    let prefs: u::Preferences = serde_json::from_value(json!({"future":{"kept":true}})).unwrap();
    assert_eq!(
        serde_json::to_value(prefs).unwrap()["future"],
        json!({"kept":true})
    );
}
#[test]
fn whats_new_defers_until_free_window_and_marks_on_show_not_download() {
    let mut model = u::WhatsNew::default();
    model.changed_version(Some("1.0"), "2.0");
    assert!(!model.show("2.0", false, true, false, false));
    assert!(!model.seen.contains("2.0"));
    assert!(!model.show("2.0", true, true, true, false));
    assert!(!model.show("2.0", true, true, false, true));
    assert!(model.show("2.0", true, true, false, false));
    assert!(model.seen.contains("2.0"));
    assert!(!model.show("2.0", true, true, false, false));
    model.dismiss();
    assert!(!model.show("2.0", true, true, false, false));
    model.changed_version(Some("2.0"), "2.0");
    assert!(!model.show("3.0", true, true, false, false));
}
#[test]
fn n004_scopes_skip_hidden_and_playback_frames_and_never_guess_stage() {
    let mut scopes = n::Scopes::new(Some(n::Scope::Waveform), true, true);
    assert_eq!(scopes.footer(), "Output · Rec. 2020");
    scopes.state = n::FrameState::Ready;
    assert!(!scopes.request_frame(0, true));
    scopes.visible = true;
    assert!(scopes.request_frame(0, true));
    assert!(!scopes.request_frame(99, true));
    assert!(scopes.request_frame(100, true));
    assert!(scopes.extended());
    scopes.state = n::FrameState::Waiting;
    assert!(scopes.keep_previous());
    scopes.state = n::FrameState::NonFinite;
    assert_eq!(scopes.error(), Some("This frame has invalid values."));
    assert!(!scopes.keep_previous());
    let mut eight_bit = n::Scopes::new(None, false, false);
    eight_bit.visible = true;
    eight_bit.state = n::FrameState::Ready;
    assert!(!eight_bit.extended());
    assert!(!eight_bit.request_frame(0, true));
    assert_eq!(eight_bit.range, n::Range::Standard);
}
#[test]
fn n005_task_transitions_eta_and_retention_keep_owner_facts() {
    assert_eq!(
        n::task_actions(n::TaskState::Queued),
        &[n::TaskAction::Cancel]
    );
    assert!(n::task_actions(n::TaskState::Pausing).is_empty());
    assert!(n::task_actions(n::TaskState::Cancelling).is_empty());
    assert_eq!(
        n::task_actions(n::TaskState::Paused),
        &[n::TaskAction::Resume, n::TaskAction::Cancel]
    );
    assert_eq!(
        n::task_actions(n::TaskState::Failed),
        &[n::TaskAction::Retry]
    );
    assert_eq!(n::task_progress(1, 0), None);
    assert!(!n::show_eta(n::TaskState::Running, 4999));
    assert!(n::show_eta(n::TaskState::Running, 5000));
    assert!(!n::show_eta(n::TaskState::Paused, 9000));
    assert!(!n::retain_task(n::TaskState::Completed, false, 5000));
    assert!(n::retain_task(n::TaskState::Completed, true, 10000));
}
#[test]
fn n005_storage_never_discards_referenced_or_sole_playable_media() {
    assert!(!n::may_archive(1, false));
    assert!(!n::may_archive(0, true));
    assert!(n::may_archive(0, false));
    assert_eq!(
        n::archive_checked_by_default(n::StorageClass::Originals),
        Some(false)
    );
    assert_eq!(
        n::archive_checked_by_default(n::StorageClass::Proxies),
        None
    );
    let archive = n::StorageUsage::new(n::StorageClass::Archive, 100, Some(1));
    assert_eq!(archive.quota, None);
    assert_eq!(
        n::StorageUsage::new(n::StorageClass::Renders, 100, Some(50)).overage(),
        50
    );
    assert!(!n::may_set_quota(n::StorageClass::Originals));
    assert!(
        n::LocationRefusal::Overlap
            .text("Renders")
            .contains("Choose a separate folder.")
    );
}
#[test]
fn n006_commands_preserve_shipped_key_fallthrough_and_atomic_refusals() {
    for key in [n::ShuttleKey::J, n::ShuttleKey::K, n::ShuttleKey::L] {
        assert!(!n::claim_shuttle(key, false, true));
        assert!(!n::claim_shuttle(key, true, false));
        assert!(n::claim_shuttle(key, true, true));
    }
    for action in [
        n::SourceAction::MarkIn,
        n::SourceAction::MarkOut,
        n::SourceAction::Insert,
        n::SourceAction::Overwrite,
    ] {
        assert!(action.key().starts_with("alt+"));
    }
    assert!(!n::SourceResolution::DurationsDisagree.may_commit());
    assert!(!n::SourceResolution::NeedsMark.may_commit());
    assert!(n::SourceResolution::Resolved.may_commit());
    assert_eq!(n::EditTool::Ripple.key(), "alt+r");
    assert_eq!(
        n::EditRefusal::BrokenLink.text(),
        "This clip’s link is damaged. Unlink it first."
    );
}
#[test]
fn n007_forbidden_capture_requests_never_route_to_system_prompts() {
    for capability in [
        n::Capability::Camera,
        n::Capability::Microphone,
        n::Capability::Screen,
    ] {
        assert_eq!(
            n::authorization_route(capability),
            n::AuthorizationRoute::RefuseExtension
        );
    }
    assert_eq!(
        n::authorization_route(n::Capability::MediaBookmark),
        n::AuthorizationRoute::Relink
    );
    assert_eq!(
        n::authorization_route(n::Capability::ProjectBookmark),
        n::AuthorizationRoute::FailedOpen
    );
    assert_eq!(
        n::authorization_route(n::Capability::Keychain),
        n::AuthorizationRoute::KeychainConsent
    );
    for (key, _) in n::USAGE_STRINGS {
        assert!(!key.contains("Camera") && !key.contains("Microphone") && !key.contains("Screen"));
    }
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
fn command_w_cancels_verification_but_never_discards_ready_update() {
    let mut model = u::Model::new("1.0".into());
    model.state = u::State::Verifying;
    model.manual = true;
    assert!(model.view().unwrap().buttons.is_empty());
    assert!(model.close_window());
    assert_eq!(model.state, u::State::Idle);
    model.state = u::State::Ready;
    assert!(!model.close_window());
    assert_eq!(model.state, u::State::Ready);
}
