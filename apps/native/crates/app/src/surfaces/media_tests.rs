use super::*;
use cloud::{Cloud, File as CloudFile, FileState};
use import::{File as ImportFile, Import, Origin, Reason, State};
use relink::{Metadata, Ratio};

fn target(id: &str) -> Target {
    Target {
        owner: Owner {
            window: "window".into(),
            project: "project".into(),
            document: 1,
            revision: 4,
        },
        id: id.into(),
        generation: 1,
    }
}
fn metadata() -> Metadata {
    Metadata {
        duration: Ratio {
            numerator: 12,
            denominator: 1,
        },
        width: 1920,
        height: 1080,
        fps: Ratio {
            numerator: 30,
            denominator: 1,
        },
        has_audio: true,
    }
}
fn alert(id: &str) -> Alert {
    relink::others(target(id), 2, "Shots").unwrap()
}
fn cloud(n: usize, automatic: bool) -> Cloud {
    Cloud::new(
        target("cloud"),
        "iCloud".into(),
        (0..n)
            .map(|i| CloudFile {
                id: i.to_string(),
                name: format!("{i}.mov"),
                state: FileState::InCloud,
            })
            .collect(),
        automatic,
    )
    .unwrap()
}
fn imports(origin: Origin, n: usize) -> Import {
    Import::new(
        target("import"),
        origin,
        (0..n)
            .map(|i| ImportFile {
                job: i.to_string(),
                name: format!("{i}.mov"),
                asset: format!("asset-{i}"),
                state: State::Queued,
                proxy_failed: false,
            })
            .collect(),
    )
    .unwrap()
}
fn sequence() -> sequence::Sequence {
    sequence::Sequence::new(
        target("seq"),
        vec![(1, "001.png".into()), (120, "120.png".into())],
        false,
        30.0,
        false,
        false,
    )
    .unwrap()
}
#[test]
fn exact_metadata_and_equivalent_rational_timebases_match() {
    let a = metadata();
    let mut b = a.clone();
    b.fps = Ratio {
        numerator: 60,
        denominator: 2,
    };
    b.duration = Ratio {
        numerator: 12000,
        denominator: 1000,
    };
    assert_eq!(
        relink::check(target("a"), "a.mov", &a, &b),
        Some(relink::Check::Match)
    );
    b.duration.numerator += 1;
    assert!(matches!(
        relink::check(target("a"), "a.mov", &a, &b),
        Some(relink::Check::Mismatch(_))
    ));
}
#[test]
fn mismatch_copy_is_capped_at_three_differences_and_buttons_keep_order() {
    let mut chosen = metadata();
    chosen.duration.numerator = 8;
    chosen.width = 1280;
    chosen.height = 720;
    chosen.fps.numerator = 25;
    chosen.has_audio = false;
    let Some(relink::Check::Mismatch(a)) =
        relink::check(target("a"), "Clip.mov", &chosen, &metadata())
    else {
        panic!()
    };
    assert_eq!(a.message, "“Clip.mov” doesn’t match the missing media.");
    assert_eq!(
        a.informative,
        "It’s 0:08 long; the original was 0:12. It’s 1280×720; the original was 1920×1080. It’s 25 fps; the original was 30 fps."
    );
    assert_eq!(
        a.buttons,
        [Action::UseAnyway, Action::Cancel, Action::ChooseAnother]
    );
    assert_eq!(a.key(Key::Return), Some(Action::UseAnyway));
    assert_eq!(a.key(Key::Escape), Some(Action::Cancel));
}
#[test]
fn audio_mismatch_and_unspecified_positive_audio_case() {
    let mut chosen = metadata();
    chosen.has_audio = false;
    let Some(relink::Check::Mismatch(a)) = relink::check(target("a"), "a", &chosen, &metadata())
    else {
        panic!()
    };
    assert_eq!(a.informative, "It has no audio; the original did.");
    assert_eq!(
        relink::check(target("a"), "a", &metadata(), &chosen),
        Some(relink::Check::NeedsDesign)
    );
}
#[test]
fn invalid_probe_never_looks_like_a_match() {
    let mut a = metadata();
    a.fps.denominator = 0;
    assert_eq!(relink::check(target("a"), "a", &a, &metadata()), None);
}
#[test]
fn relink_responses_only_request_explicit_choices() {
    assert!(relink::respond(target("a"), "new.mov".into(), Action::Cancel).is_none());
    assert_eq!(
        relink::respond(target("a"), "new.mov".into(), Action::UseAnyway)
            .unwrap()
            .operation,
        Operation::Relink {
            chosen: "new.mov".into()
        }
    );
    assert_eq!(
        relink::respond(target("a"), "new.mov".into(), Action::ChooseAnother)
            .unwrap()
            .operation,
        Operation::Locate
    );
}
#[test]
fn folder_relink_and_proxy_warning_exact_copy() {
    let a = alert("a");
    assert_eq!(a.message, "Relink 2 other missing files in “Shots”?");
    assert_eq!(
        a.informative,
        "They have the same names as missing media in this project."
    );
    assert_eq!(a.buttons, [Action::RelinkAll, Action::JustThisOne]);
    assert!(relink::others(target("a"), 0, "Shots").is_none());
    assert_eq!(
        relink::proxy_only(),
        relink::ProxyOnly {
            overlay: "Proxy only",
            symbol: "exclamationmark.triangle.fill",
            colour: "c.warning",
            meta: "Original missing · using proxy"
        }
    );
    assert_eq!(
        relink::export_warning(2),
        Some((
            "2 clips use proxies because their originals are missing.".into(),
            "Relink…"
        ))
    );
    assert!(relink::export_warning(0).is_none());
}
#[test]
fn alert_queue_is_fifo_per_window_and_duplicate_callbacks_are_fenced() {
    let mut q = AlertQueue::default();
    let mut host = InspectHost::default();
    q.push(alert("a"), &mut host);
    q.push(alert("a"), &mut host);
    q.push(alert("b"), &mut host);
    let mut other = alert("c");
    other.target.owner.window = "other-window".into();
    q.push(other, &mut host);
    assert_eq!(host.alerts.len(), 2);
    assert!(!q.finish(&target("b"), &mut host));
    assert!(q.finish(&target("a"), &mut host));
    assert!(!q.finish(&target("a"), &mut host));
    assert_eq!(host.alerts.last().unwrap().target.id, "b");
    q.close_document(&target("b").owner, &mut host);
    assert!(!q.finish(&target("b"), &mut host));
}
#[test]
fn cloud_consent_lists_eight_and_remainder_with_exact_copy() {
    let c = cloud(10, false);
    let a = c.prompt().unwrap();
    assert_eq!(a.message, "Download 10 cloud media files?");
    assert_eq!(
        a.informative,
        "These files are stored in the cloud and need to be downloaded to use them in this project.\n\n0.mov\n1.mov\n2.mov\n3.mov\n4.mov\n5.mov\n6.mov\n7.mov\n…and 2 more"
    );
    assert_eq!(
        a.suppression,
        Some("Automatically download cloud media when opening projects")
    );
    assert_eq!(a.buttons, [Action::Download, Action::NotNow]);
    assert_eq!(
        cloud(1, false).prompt().unwrap().message,
        "Download cloud media?"
    );
}
#[test]
fn cloud_never_starts_without_consent_and_not_now_is_once_per_open() {
    let mut c = cloud(1, false);
    let mut h = InspectHost::default();
    assert!(!c.start(&mut h));
    assert!(h.commands.is_empty());
    let b = c
        .respond(
            &target("cloud"),
            Action::NotNow,
            false,
            &mut cloud::CloudNotices::default(),
            &mut h,
        )
        .unwrap();
    assert_eq!(b.text, "Some media is still in the cloud.");
    assert_eq!(b.action, Some(Action::Download));
    assert_eq!(b.auto_dismiss_seconds(), None);
    assert_eq!(b.close_label(), Some("Dismiss"));
    assert!(
        c.respond(
            &target("cloud"),
            Action::NotNow,
            false,
            &mut cloud::CloudNotices::default(),
            &mut h
        )
        .is_none()
    );
    assert!(h.commands.is_empty());
    assert_eq!(c.files()[0].state, FileState::InCloud);
    assert!(c.download_action(&target("cloud"), &mut h));
}
#[test]
fn automatic_download_and_checkbox_preference_use_distinct_commands() {
    let mut c = cloud(1, true);
    let mut h = InspectHost::default();
    assert!(c.prompt().is_none());
    assert!(c.start(&mut h));
    assert!(!c.start(&mut h));
    let mut c = cloud(1, false);
    c.respond(
        &target("cloud"),
        Action::Download,
        true,
        &mut cloud::CloudNotices::default(),
        &mut h,
    );
    assert_eq!(
        h.commands[1].operation,
        Operation::SetAutomaticCloudDownload
    );
    assert!(matches!(
        h.commands[2].operation,
        Operation::Download { .. }
    ));
    assert!(c.prompt().is_none());
}
#[test]
fn cloud_progress_is_measured_monotone_and_raw_provider_errors_are_absent() {
    let mut c = cloud(2, true);
    let mut h = InspectHost::default();
    c.start(&mut h);
    assert_eq!(c.task_title(), "Downloading 2 files from iCloud");
    assert_eq!(
        Cloud::card(&c.files()[0].state),
        ("Downloading…", None, None)
    );
    assert!(c.observe(
        &target("cloud"),
        "0",
        FileState::Downloading {
            bytes: 5,
            total: Some(10)
        }
    ));
    assert_eq!(Cloud::card(&c.files()[0].state).2, Some(0.5));
    assert!(!c.observe(
        &target("cloud"),
        "0",
        FileState::Downloading {
            bytes: 4,
            total: Some(10)
        }
    ));
    assert!(!c.observe(
        &target("cloud"),
        "0",
        FileState::Downloading {
            bytes: 11,
            total: Some(10)
        }
    ));
    assert!(c.observe(&target("cloud"), "0", FileState::Downloaded));
    assert!(c.observe(&target("cloud"), "1", FileState::Failed));
    assert_eq!(c.task_title(), "1 of 2 downloaded");
    assert_eq!(
        Cloud::card(&FileState::Failed),
        ("Download failed", Some("Retry Download"), None)
    );
    assert!(c.retry(&mut h));
    assert_eq!(
        h.commands.last().unwrap().operation,
        Operation::Download {
            assets: vec!["1".into()]
        }
    );
    assert!(!c.observe(&target("cloud"), "1", FileState::Downloaded));
}
#[test]
fn cloud_cancel_keeps_finished_and_fences_late_completions() {
    let mut c = cloud(2, true);
    let mut h = InspectHost::default();
    c.start(&mut h);
    c.observe(&target("cloud"), "0", FileState::Downloaded);
    assert!(c.cancel(&mut h));
    assert!(!c.cancel(&mut h));
    assert_eq!(c.files()[0].state, FileState::Downloaded);
    assert_eq!(c.files()[1].state, FileState::InCloud);
    assert!(!c.observe(&target("cloud"), "1", FileState::Downloaded));
}
#[test]
fn cloud_rejects_empty_and_duplicate_asset_ids() {
    assert!(Cloud::new(target("c"), "p".into(), vec![], false).is_none());
    let f = CloudFile {
        id: "a".into(),
        name: "a".into(),
        state: FileState::InCloud,
    };
    assert!(Cloud::new(target("c"), "p".into(), vec![f.clone(), f], false).is_none());
}
#[test]
fn every_source_failure_has_specified_copy_and_action() {
    use source::{Cause, Source};
    for (cause, line, button) in [
        (
            Cause::Unsupported(Some("AV1".into())),
            "Powermove can’t decode AV1 files.",
            None,
        ),
        (
            Cause::Unsupported(None),
            "Powermove can’t decode this format.",
            None,
        ),
        (Cause::Damaged, "The file may be damaged.", None),
        (Cause::Offline, "The file is missing.", Some(Action::Locate)),
        (
            Cause::Cloud("iCloud".into()),
            "The file is stored in iCloud.",
            Some(Action::Download),
        ),
    ] {
        let s = Source::new(target("source"), "Clip.mov", cause);
        assert_eq!(s.title.as_deref(), Some("Can’t Preview “Clip.mov”"));
        assert_eq!(s.line.as_deref(), Some(line));
        assert_eq!(s.button, button);
        assert_eq!(s.empty_style().unwrap().0, "exclamationmark.triangle");
    }
}
#[test]
fn midplayback_decode_keeps_image_and_audio_failure_keeps_video() {
    let s = source::Source::new(target("source"), "a", source::Cause::DecodeDuringPlayback);
    assert!(s.keep_last_good_frame);
    assert!(s.title.is_none());
    assert_eq!(
        s.banner.unwrap(),
        Banner {
            text: "Couldn’t decode part of this clip.".into(),
            severity: Severity::Warning,
            action: None
        }
    );
    let s = source::Source::new(target("source"), "a", source::Cause::AudioUnreadable);
    assert!(s.video_continues);
    assert!(s.title.is_none());
    assert_eq!(s.banner.unwrap().text, "This clip’s audio can’t be played.");
}
#[test]
fn source_actions_reject_another_source_epoch_and_undeclared_action() {
    let s = source::Source::new(target("source"), "a", source::Cause::Offline);
    assert!(s.activate(&target("other"), Action::Locate).is_none());
    assert!(s.activate(&target("source"), Action::Download).is_none());
    assert_eq!(
        s.activate(&target("source"), Action::Locate)
            .unwrap()
            .operation,
        Operation::Locate
    );
}
#[test]
fn import_origins_progress_and_substeps_are_exact() {
    assert_eq!(imports(Origin::Files, 2).row().title, "Importing 2 files");
    assert_eq!(
        imports(Origin::Folder("Shots".into()), 2).row().title,
        "Importing “Shots”"
    );
    let mut i = imports(Origin::Paste, 2);
    assert_eq!(i.row().title, "Pasting media");
    assert_eq!(i.row().detail.as_deref(), Some("0 of 2"));
    i.set_step(
        &target("import"),
        Some(import::Step::Converting("a.mov".into())),
    );
    assert_eq!(i.row().detail.as_deref(), Some("Converting “a.mov”…"));
    i.set_step(&target("import"), Some(import::Step::Proxy("a.mov".into())));
    assert_eq!(
        i.row().detail.as_deref(),
        Some("Creating proxy for “a.mov”…")
    );
}
#[test]
fn import_cancel_esc_only_when_focused_retains_completed_as_one_undo() {
    let mut i = imports(Origin::Files, 3);
    let mut h = InspectHost::default();
    i.observe(&target("import"), "0", State::Imported, false, &mut h);
    assert!(!i.key(&target("import"), Key::Escape, false, &mut h));
    assert!(i.key(&target("import"), Key::Escape, true, &mut h));
    assert_eq!(i.row().title, "Import stopped · 1 of 3 imported");
    assert_eq!(i.files()[0].state, State::Imported);
    assert_eq!(
        h.commands[0].operation,
        Operation::CancelRemaining {
            jobs: vec!["1".into(), "2".into()],
            undo_label: Some("Import Media")
        }
    );
    assert!(!i.observe(&target("import"), "1", State::Imported, false, &mut h));
    assert!(i.row().banner.is_none());
}
#[test]
fn import_success_completes_quietly_without_retry() {
    let mut i = imports(Origin::Files, 1);
    let mut h = InspectHost::default();
    i.observe(&target("import"), "0", State::Imported, false, &mut h);
    assert!(i.terminal());
    assert!(!i.row().cancel);
    assert!(!i.row().retry);
    assert!(i.row().banner.is_none());
    assert!(!i.retry(&target("import"), &mut h));
}
#[test]
fn partial_import_retry_excludes_unsupported_successful_and_cancelled_files() {
    let mut i = imports(Origin::Files, 3);
    let mut h = InspectHost::default();
    i.observe(&target("import"), "0", State::Imported, false, &mut h);
    i.observe(
        &target("import"),
        "1",
        State::Failed(Reason::Unsupported),
        false,
        &mut h,
    );
    i.observe(
        &target("import"),
        "2",
        State::Failed(Reason::Damaged),
        false,
        &mut h,
    );
    let row = i.row();
    assert_eq!(row.title, "1 of 3 files imported");
    assert_eq!(row.severity, Severity::Warning);
    assert_eq!(
        row.disclosure,
        [
            ("1.mov".into(), "Not a supported format".into()),
            ("2.mov".into(), "The file may be damaged".into())
        ]
    );
    assert!(i.retry(&target("import"), &mut h));
    assert_eq!(
        h.commands[0].operation,
        Operation::RetryImports {
            jobs: vec!["2".into()]
        }
    );
    assert_eq!(i.files()[0].state, State::Imported);
    assert_eq!(i.files()[1].state, State::Failed(Reason::Unsupported));
}
#[test]
fn no_import_has_error_row_and_persistent_details_banner() {
    let mut i = imports(Origin::Files, 1);
    let mut h = InspectHost::default();
    i.observe(
        &target("import"),
        "0",
        State::Failed(Reason::Missing),
        false,
        &mut h,
    );
    let row = i.row();
    assert_eq!(row.title, "Couldn’t import 1 files");
    assert_eq!(row.severity, Severity::Error);
    assert_eq!(
        row.banner.unwrap(),
        Banner {
            text: "No media was imported.".into(),
            severity: Severity::Warning,
            action: Some(Action::ShowDetails)
        }
    );
}
#[test]
fn proxy_failure_retains_original_and_retries_proxy_only() {
    let mut i = imports(Origin::Files, 1);
    let mut h = InspectHost::default();
    i.observe(&target("import"), "0", State::Imported, true, &mut h);
    assert_eq!(
        i.row().disclosure[0].1,
        "Couldn’t create a proxy for “0.mov”."
    );
    assert_eq!(i.files()[0].state, State::Imported);
    assert!(i.retry(&target("import"), &mut h));
    assert_eq!(
        h.commands[0].operation,
        Operation::RetryProxies {
            assets: vec!["asset-0".into()]
        }
    );
}
#[test]
fn disk_full_uses_service_copy_and_stops_remaining_files() {
    let mut i = imports(Origin::Files, 2);
    let mut h = InspectHost::default();
    i.observe(
        &target("import"),
        "0",
        State::Failed(Reason::DiskFull),
        false,
        &mut h,
    );
    assert_eq!(i.row().title, "There isn’t enough disk space.");
    assert_eq!(
        i.row().detail.as_deref(),
        Some("Free some space and try again.")
    );
    assert_eq!(i.files()[1].state, State::Cancelled);
    assert_eq!(i.row().severity, Severity::Error);
    assert_eq!(
        Reason::Permission.label(),
        "You don’t have permission to read it"
    );
}
#[test]
fn import_observations_reject_duplicate_and_stale_results() {
    let mut i = imports(Origin::Files, 1);
    let mut h = InspectHost::default();
    let mut stale = target("import");
    stale.owner.document += 1;
    assert!(!i.observe(&stale, "0", State::Imported, false, &mut h));
    assert!(i.observe(
        &target("import"),
        "0",
        State::Failed(Reason::Damaged),
        false,
        &mut h
    ));
    assert!(i.retry(&target("import"), &mut h));
    assert!(!i.observe(&target("import"), "0", State::Imported, false, &mut h));
    let next = i.target().clone();
    assert!(i.observe(&next, "0", State::Imported, false, &mut h));
    assert!(!i.observe(&next, "0", State::Failed(Reason::Missing), false, &mut h));
}
#[test]
fn sequence_form_and_copy_follow_system_sheet_grammar() {
    let s = sequence();
    assert_eq!(s.title(), "Import Image Sequence");
    assert_eq!(s.explanation(), "Turn your numbered images into one clip.");
    assert_eq!(s.image_count(), "2 images");
    assert_eq!(s.range(), "001.png to 120.png");
    assert_eq!(s.duration().as_deref(), Some("0:04 · 120 frames"));
    assert_eq!(
        s.warnings(),
        ["118 numbers are missing from the sequence; those frames hold the previous image."]
    );
    assert_eq!(
        s.buttons(),
        [
            Action::IndividualImages,
            Action::Cancel,
            Action::ImportSequence
        ]
    );
    assert_eq!(sequence::Sequence::fields(), ["Frame Rate", "Duration"]);
    assert_eq!(
        sequence::Sequence::checking(3, 120),
        "Checking frames · 3 of 120"
    );
    assert_eq!(sequence::Sequence::creating(), "Creating image sequence…");
}
#[test]
fn sequence_live_validation_fences_return_for_invalid_fps() {
    let mut s = sequence();
    for fps in [0.0, 0.9, 240.1, f64::NAN, f64::INFINITY] {
        s.fps = fps;
        assert_eq!(
            s.validation(),
            Some("Enter a frame rate from 1 to 240 fps.")
        );
        assert_eq!(s.duration(), None);
        assert_eq!(s.key(Key::Return), None);
        assert!(s.respond(&target("seq"), Action::ImportSequence).is_none());
    }
    for fps in [1.0, 23.976, 240.0] {
        s.fps = fps;
        assert!(s.validation().is_none());
        assert_eq!(s.key(Key::Return), Some(Action::ImportSequence));
    }
    assert_eq!(s.key(Key::Escape), Some(Action::Cancel));
}
#[test]
fn sequence_replace_and_required_callers_have_no_individual_images() {
    let mut s = sequence();
    s.replace = true;
    s.mixed_sizes = true;
    assert_eq!(s.title(), "Replace Image Sequence");
    assert_eq!(s.buttons(), [Action::Cancel, Action::Replace]);
    assert_eq!(
        s.warnings()[1],
        "The images aren’t all the same size; each is fitted to the first."
    );
    assert!(
        s.respond(&target("seq"), Action::IndividualImages)
            .is_none()
    );
    let mut s = sequence();
    s.required = true;
    assert_eq!(s.buttons(), [Action::Cancel, Action::ImportSequence]);
}
#[test]
fn sequence_cancel_stale_and_double_submit_never_admit_work() {
    let mut s = sequence();
    assert!(
        s.respond(&target("other"), Action::ImportSequence)
            .is_none()
    );
    assert!(matches!(
        s.respond(&target("seq"), Action::ImportSequence)
            .unwrap()
            .operation,
        Operation::Sequence {
            fps: 30.0,
            replace: false,
            ..
        }
    ));
    assert!(s.respond(&target("seq"), Action::ImportSequence).is_none());
    let mut s = sequence();
    s.respond(&target("seq"), Action::Cancel);
    assert!(s.respond(&target("seq"), Action::ImportSequence).is_none());
    let mut s = sequence();
    s.target_disappeared(None);
    assert!(s.respond(&target("seq"), Action::ImportSequence).is_none());
}
#[test]
fn sequence_rejects_invalid_frame_span() {
    assert!(sequence::Sequence::new(target("s"), vec![], false, 30.0, false, false).is_none());
    assert!(
        sequence::Sequence::new(
            target("s"),
            vec![(2, "1".into()), (1, "2".into())],
            false,
            30.0,
            false,
            false
        )
        .is_none()
    );
}
#[test]
fn figma_supported_selection_pastes_without_prompt() {
    assert_eq!(
        figma::present(target("f"), figma::Clipboard::Supported),
        figma::Presentation::PasteEditable
    );
}
#[test]
fn figma_fallback_available_is_whole_selection_and_requires_consent() {
    let state = figma::Clipboard::Fallback {
        reasons: vec!["Masks with blur can’t stay editable.".into()],
        preview: true,
    };
    let figma::Presentation::Alert(a) = figma::present(target("f"), state.clone()) else {
        panic!()
    };
    assert_eq!(a.message, "Paste the Figma selection as an image?");
    assert_eq!(
        a.informative,
        "Masks with blur can’t stay editable. The whole selection becomes one image you can animate; its parts won’t be editable."
    );
    assert_eq!(a.buttons, [Action::PasteImage, Action::Cancel]);
    assert!(figma::respond(target("f"), &state, Action::Cancel).is_none());
    assert_eq!(
        figma::respond(target("f"), &state, Action::PasteImage)
            .unwrap()
            .operation,
        Operation::PasteFigmaImage
    );
}
#[test]
fn figma_no_preview_is_acknowledgement_and_reasons_cap_at_four() {
    let state = figma::Clipboard::Fallback {
        reasons: vec!["Masks with blur can’t stay editable.".into(); 6],
        preview: false,
    };
    let figma::Presentation::Alert(a) = figma::present(target("f"), state.clone()) else {
        panic!()
    };
    assert_eq!(a.message, "These Figma layers need an image fallback.");
    assert!(a.informative.ends_with("and 2 more Figma didn’t include a preview image. Use Copy as PNG in Figma, then paste again."));
    assert_eq!(
        a.informative
            .matches("Masks with blur can’t stay editable.")
            .count(),
        4
    );
    assert_eq!(a.buttons, [Action::Ok]);
    assert_eq!(a.key(Key::Return), Some(Action::Ok));
    assert!(figma::respond(target("f"), &state, Action::PasteImage).is_none());
}
#[test]
fn unreadable_figma_banner_has_warning_copy_and_system_announcement() {
    let figma::Presentation::Banner(b) = figma::present(target("f"), figma::Clipboard::Unreadable)
    else {
        panic!()
    };
    assert_eq!(
        b.text,
        "Couldn’t read the Figma selection. Copy it again in Figma."
    );
    assert_eq!(b.severity, Severity::Warning);
    assert_eq!(b.auto_dismiss_seconds(), Some(4));
    assert_eq!(b.announcement_priority(), "medium");
    assert_eq!(
        Banner {
            severity: Severity::Error,
            ..b
        }
        .announcement_priority(),
        "high"
    );
}
#[test]
fn actions_expose_exact_keyboard_and_voiceover_button_labels() {
    let actions = [
        Action::UseAnyway,
        Action::Cancel,
        Action::ChooseAnother,
        Action::RelinkAll,
        Action::JustThisOne,
        Action::Download,
        Action::NotNow,
        Action::Retry,
        Action::Locate,
        Action::ShowDetails,
        Action::IndividualImages,
        Action::ImportSequence,
        Action::Replace,
        Action::PasteImage,
        Action::Ok,
    ];
    assert_eq!(
        actions.map(Action::label),
        [
            "Use Anyway",
            "Cancel",
            "Choose Another…",
            "Relink All",
            "Just This One",
            "Download",
            "Not Now",
            "Retry",
            "Locate File…",
            "Show Details",
            "Individual Images",
            "Import Sequence",
            "Replace",
            "Paste as Image",
            "OK"
        ]
    );
}
#[test]
fn cloud_banner_is_once_per_document_open_across_batches_and_revisions() {
    let mut a = cloud(1, false);
    let mut b = cloud(2, false);
    let mut h = InspectHost::default();
    let mut notices = cloud::CloudNotices::default();
    assert!(
        a.respond(
            &target("cloud"),
            Action::NotNow,
            false,
            &mut notices,
            &mut h
        )
        .is_some()
    );
    assert!(
        b.respond(
            &target("cloud"),
            Action::NotNow,
            false,
            &mut notices,
            &mut h
        )
        .is_none()
    );
    notices.close_document(&target("cloud").owner);
    let mut c = cloud(1, false);
    assert!(
        c.respond(
            &target("cloud"),
            Action::NotNow,
            false,
            &mut notices,
            &mut h
        )
        .is_some()
    );
}
#[test]
fn stale_sequence_target_closes_and_uses_only_the_supplied_object_label() {
    let mut s = sequence();
    assert_eq!(
        s.target_disappeared(Some("Composition")).unwrap().text,
        "Composition was changed, so the dialog closed."
    );
    assert!(s.respond(&target("seq"), Action::ImportSequence).is_none());
    assert!(s.target_disappeared(Some("Composition")).is_none());
}
#[test]
fn relink_all_skips_mismatching_metadata_and_deduplicates_asset_ids() {
    let mut wrong = metadata();
    wrong.width = 1280;
    let (command, skipped) = relink::matching_folder(
        target("folder"),
        "Shots".into(),
        vec![
            relink::FolderCandidate {
                id: "match".into(),
                name: "a.mov".into(),
                recorded: metadata(),
                probed: metadata(),
            },
            relink::FolderCandidate {
                id: "mismatch".into(),
                name: "b.mov".into(),
                recorded: metadata(),
                probed: wrong,
            },
            relink::FolderCandidate {
                id: "match".into(),
                name: "a.mov".into(),
                recorded: metadata(),
                probed: metadata(),
            },
        ],
    );
    assert_eq!(
        command.unwrap().operation,
        Operation::RelinkMatching {
            folder: "Shots".into(),
            assets: vec!["match".into()]
        }
    );
    assert_eq!(skipped, ["mismatch"]);
}
#[test]
fn alert_interaction_is_one_shot_and_cannot_drive_a_queued_alert_early() {
    let mut queue = AlertQueue::default();
    let mut h = InspectHost::default();
    queue.push(alert("first"), &mut h);
    let a = alert("second");
    let mut interaction = AlertInteraction::new(
        a.clone(),
        vec![(
            Action::RelinkAll,
            Operation::RelinkMatching {
                folder: "Shots".into(),
                assets: vec!["a".into()],
            },
        )],
    )
    .unwrap();
    queue.push(a, &mut h);
    assert!(!interaction.respond(&target("second"), Action::RelinkAll, &mut queue, &mut h));
    assert!(h.commands.is_empty());
    queue.finish(&target("first"), &mut h);
    assert!(!interaction.respond(&target("second"), Action::UseAnyway, &mut queue, &mut h));
    assert!(interaction.respond(&target("second"), Action::RelinkAll, &mut queue, &mut h));
    assert!(!interaction.respond(&target("second"), Action::RelinkAll, &mut queue, &mut h));
    assert_eq!(h.commands.len(), 1);
}
#[test]
fn aborted_sheet_never_queues_the_default_operation() {
    let mut queue = AlertQueue::default();
    let mut h = InspectHost::default();
    let a = alert("a");
    let mut interaction =
        AlertInteraction::new(a.clone(), vec![(Action::RelinkAll, Operation::Locate)]).unwrap();
    queue.push(a, &mut h);
    assert!(interaction.abort(&mut queue, &mut h));
    assert!(!interaction.respond(&target("a"), Action::RelinkAll, &mut queue, &mut h));
    assert!(h.commands.is_empty());
}
#[test]
fn sequence_command_carries_exact_numbered_span_and_authored_gap_and_size_policies() {
    let mut s = sequence();
    let command = s.respond(&target("seq"), Action::ImportSequence).unwrap();
    assert!(
        matches!(command.operation, Operation::Sequence { frame_count: 120, hold_missing_frames: true, fit_to_first: true, ref frame_numbers, .. } if frame_numbers == &[1, 120])
    );
    assert!(
        sequence::Sequence::new(
            target("s"),
            vec![(0, "0".into()), (u64::MAX, "last".into())],
            false,
            30.0,
            false,
            false
        )
        .is_none()
    );
}
