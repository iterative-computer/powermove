use super::{windows::*, *};
fn handle(id: &str) -> SessionHandle {
    SessionHandle {
        project: id.into(),
        generation: format!("generation-{id}"),
    }
}
fn doc(id: &str, dirty: bool) -> Document {
    Document {
        session: handle(id),
        window: 1,
        name: id.into(),
        dirty,
        saving: false,
        importing: false,
        entry: false,
        has_file: true,
        copy: false,
        disk_changed: false,
    }
}
fn request(ticket: u64, window: u64, session: SessionHandle) -> Request {
    Request {
        ticket: Ticket(ticket),
        target: Target {
            owner: Owner::Window(window),
            session: Some(session),
        },
        presentation: Presentation::Alert(unsaved("Film")),
    }
}
#[test]
fn close_copy_and_keyboard_are_exact() {
    let a = unsaved("Launch Film");
    assert_eq!(a.message, "Save changes to “Launch Film” before closing?");
    assert_eq!(
        a.informative,
        "Your project file hasn’t been updated. Local recovery is kept separately."
    );
    assert_eq!(
        a.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Save", "Cancel", "Don’t Save"]
    );
    assert_eq!(a.action_for_key(Key::Return), Some(Action::Save));
    assert_eq!(a.action_for_key(Key::Escape), Some(Action::Cancel));
    assert_eq!(a.action_for_key(Key::CommandD), Some(Action::DontSave));
    assert!(a.buttons[2].destructive);
    assert_eq!(a.action_at(None), Some(Action::Cancel));
    assert_eq!(a.action_at(Some(999)), Some(Action::Cancel));
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
    assert_eq!(
        a.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Review Changes…", "Cancel", "Discard Changes"]
    );
    assert_eq!(a.action_for_key(Key::Return), Some(Action::ReviewChanges));
    assert!(a.buttons[2].destructive);
    assert_eq!(a.buttons[2].key, None);
}
#[test]
fn changed_disk_copy_is_exact() {
    let a = changed_on_disk("Film");
    assert_eq!(
        a.message,
        "“Film” was changed by another app since you opened or saved it."
    );
    assert_eq!(
        a.informative,
        "If you save, the other version is kept as a recovery copy."
    );
    assert_eq!(
        a.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Save Anyway", "Cancel", "Save As…"]
    );
}
#[test]
fn service_error_table_is_exact_and_diagnostics_never_become_copy() {
    for (code, message, info) in [
        (
            "offline",
            "Powermove can’t connect.",
            "Check your internet connection and try again.",
        ),
        (
            "not_found",
            "“Film” can’t be found.",
            "It may have been moved or deleted.",
        ),
        ("too_large", "“Film” is too large to open.", ""),
        (
            "invalid_project",
            "“Film” can’t be opened because it isn’t a Powermove project.",
            "",
        ),
        (
            "permission_denied",
            "You don’t have permission to open “Film”.",
            "Check its sharing and permissions in Finder.",
        ),
        (
            "disk_full",
            "There isn’t enough disk space.",
            "Free some space and try again.",
        ),
        (
            "invalid_input",
            "“Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            "invalid_data",
            "“Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            "unavailable",
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            "unknown",
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            "diagnostic / private/path",
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    ] {
        assert_eq!(service_error(code, "Film"), (message.into(), info));
    }
}
#[test]
fn open_errors_only_offer_finder_for_existing_paths_and_ok_is_default() {
    for exists in [true, false] {
        let a = open_error("Film", "invalid_data", exists);
        assert_eq!(a.action_for_key(Key::Return), Some(Action::Ok));
        assert_eq!(
            a.buttons.iter().any(|b| b.action == Action::ShowInFinder),
            exists
        );
        assert_eq!(a.buttons[0].label, "OK");
    }
}
#[test]
fn every_copy_reason_is_exact_and_has_a_persistent_save_as_action() {
    for (reason, line) in [
        (
            CopyReason::Locked,
            "“Film” is open in Powermove on another Mac, so this is a copy.",
        ),
        (
            CopyReason::UncertainLock,
            "“Film” may be open elsewhere, so this is a copy.",
        ),
        (
            CopyReason::ReadOnly,
            "“Film” is read-only, so this is a copy.",
        ),
        (
            CopyReason::CloudFolder,
            "“Film” is in a cloud folder that’s still syncing, so this is a copy.",
        ),
        (
            CopyReason::OfflineFile,
            "“Film” isn’t downloaded, so this is a copy of the last local version.",
        ),
        (
            CopyReason::ChangedOnDisk,
            "“Film” changed while it was opening, so this is a copy.",
        ),
        (
            CopyReason::OlderElectron,
            "“Film” is open in an older Powermove, so this is a copy.",
        ),
        (
            CopyReason::ForeignLibrary,
            "“Film” belongs to another Powermove library, so this is a copy.",
        ),
    ] {
        let n = copy_notice("Film", &OpenDecision::AsCopy(reason)).unwrap();
        assert_eq!(n.text, line);
        assert_eq!(n.action, Some((Action::SaveAs, "Save As…")));
        assert_eq!(n.viewer_timeout(), None);
        assert_eq!(n.dismiss_label(), Some("Dismiss"));
    }
    assert!(copy_notice("Film", &OpenDecision::Writable).is_none());
}
#[test]
fn notices_have_exact_copy_durations_actions_and_ax_priority() {
    let n = reloaded("Film");
    assert_eq!(n.text, "“Film” was updated from another app.");
    assert_eq!(n.viewer_timeout(), Some(Duration::from_secs(4)));
    assert_eq!(n.priority, AnnouncementPriority::Medium);
    let n = recovery_copy("Film", "recovery-id".into());
    assert_eq!(n.text, "A recovery copy of “Film” was kept.");
    assert_eq!(n.action, Some((Action::ShowInFinder, "Show in Finder")));
    assert_eq!(n.recovery_id.as_deref(), Some("recovery-id"));
    assert_eq!(n.viewer_timeout(), None);
    let n = uncertain_save("Film");
    assert_eq!(n.text, "Couldn’t confirm that “Film” was saved.");
    assert_eq!(n.action, Some((Action::SaveAgain, "Save Again")));
    assert_eq!(n.level, Level::Warning);
    assert_eq!(n.priority, AnnouncementPriority::High);
}
#[test]
fn recovery_ids_are_retained_for_all_reply_shapes() {
    for reply in [
        ShellReply::Saved {
            session: handle("Film"),
            recovery_copy: Some("a".into()),
        },
        ShellReply::Closed {
            recovery_copy: Some("a".into()),
        },
    ] {
        assert_eq!(
            recovery_notices("Film", &reply)[0].recovery_id.as_deref(),
            Some("a")
        );
    }
    assert!(recovery_notices("Film", &ShellReply::Durable).is_empty());
}
#[test]
fn finishing_copy_and_cancel_key_are_exact() {
    for (work, line) in [
        (Work::Saving, "Powermove is saving “Film”."),
        (Work::Durability, "Powermove is saving “Film”."),
        (Work::Importing, "Powermove is importing media into “Film”."),
    ] {
        let s = Finishing::new("Film", work);
        assert_eq!(s.title, "Finishing Up…");
        assert_eq!(s.line, line);
        assert_eq!(s.cancel.label, "Cancel");
        assert_eq!(s.cancel.key, Some(Key::Escape));
    }
}
#[test]
fn picker_contracts_and_copy_names_are_exact() {
    let open = Picker::new(PickerKind::Open);
    assert_eq!((open.title, open.prompt), ("Open", "Open"));
    assert!(open.files && open.multiple && !open.folders);
    let save = Picker::save("Film", false);
    assert_eq!((save.title, save.prompt), ("Save As", "Save"));
    assert!(save.create_directories);
    assert_eq!(
        save.kind,
        PickerKind::Save {
            name: "Film".into()
        }
    );
    assert_eq!(
        Picker::save("Film", true).kind,
        PickerKind::Save {
            name: "Film copy".into()
        }
    );
    let import = Picker::new(PickerKind::Import {
        supported_types: vec!["public.movie".into()],
    });
    assert_eq!((import.title, import.prompt), ("Import", "Import"));
    assert!(import.files && import.folders && import.multiple);
    let folder = Picker::new(PickerKind::Folder);
    assert_eq!((folder.title, folder.prompt), ("Choose", "Choose"));
    assert!(folder.folders && folder.create_directories && !folder.files && !folder.multiple);
}
#[test]
fn subtitle_states_are_exact() {
    let mut d = doc("Film", false);
    assert_eq!(d.subtitle(), "");
    d.dirty = true;
    assert_eq!(d.subtitle(), "Edited");
    d.copy = true;
    assert_eq!(d.subtitle(), "Copy · Edited");
    d.dirty = false;
    assert_eq!(d.subtitle(), "Copy");
    d.saving = true;
    assert_eq!(d.subtitle(), "Saving…");
}
#[test]
fn request_tickets_are_process_unique() {
    let mut a = Tickets::default();
    let mut b = Tickets::default();
    assert_ne!(a.next_ticket(), b.next_ticket());
}
#[test]
fn alerts_are_fifo_per_window_and_independent_between_windows() {
    let mut q = PresentationQueue::default();
    let mut h = HeadlessHost::default();
    q.enqueue(request(1, 1, handle("a")), &mut h);
    q.enqueue(request(2, 1, handle("b")), &mut h);
    q.enqueue(request(3, 2, handle("c")), &mut h);
    assert_eq!(
        h.presented.iter().map(|r| r.ticket).collect::<Vec<_>>(),
        [Ticket(1), Ticket(3)]
    );
    assert!(q.complete(Ticket(9), &mut h).is_none());
    q.complete(Ticket(1), &mut h);
    assert_eq!(h.presented.last().unwrap().ticket, Ticket(2));
}
#[test]
fn retire_cancels_only_the_matching_generation_and_advances_queue() {
    let mut q = PresentationQueue::default();
    let mut h = HeadlessHost::default();
    let old = handle("a");
    let mut new = old.clone();
    new.generation = "reopened".into();
    q.enqueue(request(1, 1, old.clone()), &mut h);
    q.enqueue(request(2, 1, old.clone()), &mut h);
    q.enqueue(request(3, 1, new), &mut h);
    q.retire(&old, &mut h);
    assert_eq!(h.dismissed, [Ticket(1)]);
    assert_eq!(h.presented.last().unwrap().ticket, Ticket(3));
    assert!(q.complete(Ticket(2), &mut h).is_none());
}
#[test]
fn opening_delay_announcements_and_shortcut_cancellation_are_exact() {
    let mut o = Opening::new(Ticket(1), 2, "Film".into(), Duration::ZERO);
    assert!(!o.project_actions_enabled());
    assert!(o.tick(Duration::from_millis(499)).is_empty());
    let e = o.tick(Duration::from_millis(500));
    assert_eq!(
        e,
        [
            OpenEffect::ShowLoading {
                window: 2,
                text: "Opening “Film”…".into()
            },
            OpenEffect::Announce {
                window: 2,
                text: "Opening Film".into()
            }
        ]
    );
    assert!(o.tick(Duration::from_secs(2)).is_empty());
    assert_eq!(
        o.cancel(),
        [
            OpenEffect::CancelOpen { ticket: Ticket(1) },
            OpenEffect::CloseWindow(2)
        ]
    );
    assert!(o.cancel().is_empty());
}
#[test]
fn recovery_routes_to_s001_and_keeps_the_open_window() {
    let mut o = Opening::new(Ticket(1), 2, "Film".into(), Duration::ZERO);
    let e = o.reply(
        Ticket(1),
        &ShellReply::RecoveryRequired {
            ticket: "recovery-ticket".into(),
            recovery: pm_kernel::session::RecoveryResult::Recovered { truncated: false },
        },
    );
    assert_eq!(o.phase, OpenPhase::Recovery);
    assert!(matches!(&e[0],OpenEffect::Recovery {ticket,..} if ticket=="recovery-ticket"));
    assert!(!e.iter().any(|e| matches!(e, OpenEffect::CloseWindow(_))));
}
#[test]
fn failed_open_closes_its_tab_then_uses_remaining_window_or_app_modal() {
    for remaining in [None, Some(9)] {
        let mut o = Opening::new(Ticket(1), 2, "Film".into(), Duration::ZERO);
        let e = o.failed(Ticket(1), "not_found", false, remaining);
        assert_eq!(e[0], OpenEffect::CloseWindow(2));
        assert!(
            matches!(&e[1],OpenEffect::Error {owner,..} if *owner==remaining.map_or(Owner::Application,Owner::Window))
        );
        assert_eq!(o.phase, OpenPhase::Failed);
        assert!(o.tick(Duration::from_secs(2)).is_empty());
    }
}
#[test]
fn restore_errors_aggregate_once_and_already_open_only_focuses() {
    assert_eq!(already_open(7), OpenEffect::Focus { window: 7 });
    assert!(restore_error(&[]).is_none());
    let failed = |name: &str| RestoreFailure {
        name: name.into(),
        service_code: "not_found".into(),
        path: PathBuf::from(format!("{name}.pmv")),
        path_exists: false,
    };
    assert_eq!(
        restore_error(&[failed("a")]).unwrap().message,
        "“a” can’t be found."
    );
    let a = restore_error(&[failed("a"), failed("b"), failed("c")]).unwrap();
    assert_eq!(a.message, "Some projects couldn’t be reopened.");
    assert_eq!(
        a.informative,
        "“a”, “b” and 1 more may have been moved or deleted."
    );
    assert_eq!(
        restore_error(&[failed("a"), failed("b")])
            .unwrap()
            .informative,
        "“a” and “b” may have been moved or deleted."
    );
    assert_eq!(a.action_for_key(Key::Return), Some(Action::Ok));
    assert_eq!(a.buttons[1].label, "Show Project Library");
}
#[test]
fn saving_tab_transfer_is_deferred_and_refusal_silently_snaps_back() {
    let (mut t, e) = TabTransfer::new(handle("a"), 7, true);
    assert!(e.is_empty());
    assert!(t.waiting);
    assert!(t.observe(true).is_empty());
    assert_eq!(
        t.observe(false),
        [TransferEffect::Request {
            session: handle("a"),
            destination: 7
        }]
    );
    assert!(t.observe(false).is_empty());
    assert_eq!(t.resolved(false), [TransferEffect::SnapBack]);
    assert!(t.resolved(true).is_empty());
}
fn popout(panel: &str) -> Popout {
    Popout::new(PopoutSpec {
        panel_id: panel.into(),
        panel_title: "Properties".into(),
        project_name: "Film".into(),
        previous_dock: "old".into(),
        default_dock: "right".into(),
    })
}
#[test]
fn popout_title_accessibility_and_symbols_are_exact() {
    let p = popout("inspector");
    assert_eq!(p.spec.title(), "Properties — Film");
    assert_eq!(p.spec.return_label(), "Return Properties to Layout");
    assert_eq!(PopoutSpec::RETURN_TOOLTIP, "Return to Layout");
    assert_eq!(
        PopoutSpec::RETURN_SYMBOL,
        "rectangle.inset.filled.and.arrow.left"
    );
    assert!(p.spec.autosave_name().contains("inspector"));
}
#[test]
fn dock_only_panels_have_no_popout_action_and_failed_creation_stays_docked() {
    for id in ["viewer", "toolbar", "tools"] {
        let mut p = popout(id);
        assert!(!p.spec.eligible());
        assert!(p.open().is_empty());
    }
    let mut p = popout("inspector");
    assert!(matches!(&p.open()[0], PopoutEffect::Create(_)));
    assert!(p.created(false).is_empty());
    assert!(!p.popped_out);
    assert!(p.return_to_layout(true).is_empty());
}
#[test]
fn popout_return_uses_previous_or_default_dock_and_is_idempotent() {
    for exists in [false, true] {
        let mut p = popout("inspector");
        p.open();
        assert!(p.open().is_empty());
        p.created(true);
        assert!(p.popped_out);
        let e = p.return_to_layout(exists);
        assert!(
            matches!(&e[0],PopoutEffect::Dock {position,..} if position==if exists {"old"} else {"right"})
        );
        assert!(p.return_to_layout(exists).is_empty());
    }
}
#[test]
fn project_close_preserves_popout_restoration_without_returning_to_dock() {
    let mut p = popout("inspector");
    p.open();
    p.created(true);
    let e = p.project_closed();
    assert_eq!(
        e,
        [
            PopoutEffect::RecordPoppedOut {
                panel: "inspector".into(),
                popped_out: true
            },
            PopoutEffect::CloseWindow("inspector".into())
        ]
    );
    assert!(p.return_to_layout(true).is_empty());
}
#[test]
fn external_change_defers_dirty_decision_until_save_and_never_merges() {
    let mut c = DiskChange::default();
    assert!(!c.observed(true));
    assert!(c.before_save("Film").is_some());
    c.approved();
    assert!(c.before_save("Film").is_none());
    assert!(c.observed(false));
    assert_eq!(
        c.did_reload("Film").text,
        "“Film” was updated from another app."
    );
    assert!(c.before_save("Film").is_none());
}
// Kernel-backed tests use only synthetic owned fixtures and canonical temp roots.
fn fixture() -> (tempfile::TempDir, pm_kernel::KernelActor, SessionHandle) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
        journal_root: Some(root.join("journals")),
        ..Default::default()
    });
    let k = actor.handle();
    submit(
        &k,
        ShellCall::AttachLibrary {
            profile: root.join("profile"),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        },
    )
    .wait()
    .unwrap()
    .unwrap();
    let value=pm_model::json::parse(r##"{"id":"a","name":"Film","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap();
    let ShellReply::Opened { session, .. } = submit(
        &k,
        ShellCall::Open {
            id: "a".into(),
            source: pm_kernel::session::SessionSource::New(value),
            electron_profile: None,
        },
    )
    .wait()
    .unwrap()
    .unwrap() else {
        panic!()
    };
    submit(&k, ShellCall::Save(session.clone()))
        .wait()
        .unwrap()
        .unwrap();
    (dir, actor, session)
}
fn edit(k: &pm_kernel::KernelHandle) {
    k.call_blocking(
        "a",
        pm_kernel::Request::new(
            pm_kernel::Caller::native(),
            pm_kernel::Call::Transact {
                label: Some("Rename".into()),
                body: pm_kernel::WriteBody::Commands(vec![
                    pm_kernel::CompCommand::Set {
                        patch: Box::new(pm_kernel::commands::comps::CompPatch {
                            name: Some("Edited Film".into()),
                            ..Default::default()
                        }),
                    }
                    .into(),
                ]),
                expected_revision: None,
            },
        ),
    )
    .unwrap();
}
#[test]
fn real_kernel_publications_keep_generation_dirty_path_and_save_flight() {
    let (dir, a, s) = fixture();
    let k = a.handle();
    edit(&k);
    let p = k.published("a").unwrap();
    let d = Document::from_published(&p, 3, "Film".into(), false, &OpenDecision::Writable).unwrap();
    assert!(d.dirty);
    assert_eq!(d.session, s);
    assert!(!d.has_file);
    let flight = k
        .prepare_save(
            s.clone(),
            Some(dir.path().canonicalize().unwrap().join("Film.pmv")),
        )
        .wait()
        .unwrap()
        .unwrap();
    let p = k.published("a").unwrap();
    assert!(
        Document::from_published(&p, 3, "Film".into(), false, &OpenDecision::Writable)
            .unwrap()
            .saving
    );
    k.finish_save(flight.run()).wait().unwrap().unwrap();
    let p = k.published("a").unwrap();
    let d = Document::from_published(&p, 3, "Film".into(), false, &OpenDecision::Writable).unwrap();
    assert!(!d.dirty);
    assert!(!d.saving);
    assert!(d.has_file);
    a.shutdown();
}
#[test]
fn real_open_reply_announces_completion_once_and_late_cancel_retires_generation() {
    let (_dir, a, s) = fixture();
    let k = a.handle();
    let source=pm_kernel::session::SessionSource::New(pm_model::json::parse(r##"{"id":"b","name":"Film","compId":"c","w":320,"h":180,"fps":30,"dur":1,"bg":"#000000","layers":[],"assets":{},"comps":{}}"##).unwrap());
    let reply = submit(
        &k,
        ShellCall::PrepareOpen {
            id: "b".into(),
            source,
            electron_profile: None,
        },
    )
    .wait()
    .unwrap()
    .unwrap();
    let mut o = Opening::new(Ticket(1), 2, "Film".into(), Duration::ZERO);
    let e = o.reply(Ticket(1), &reply);
    assert!(o.project_actions_enabled());
    assert!(e.contains(&OpenEffect::Announce {
        window: 2,
        text: "Film opened".into()
    }));
    assert!(o.reply(Ticket(1), &reply).is_empty());
    let mut cancelled = Opening::new(Ticket(2), 3, "Film".into(), Duration::ZERO);
    cancelled.cancel();
    let e = cancelled.reply(Ticket(2), &reply);
    assert!(matches!(&e[0],OpenEffect::Retire(session) if session.project=="b"));
    submit(&k, ShellCall::DeferClose(s))
        .wait()
        .unwrap()
        .unwrap();
    a.shutdown();
}

#[test]
fn command_save_preflight_never_writes_a_copy_or_unsaved_original() {
    let mut d = doc("Film", true);
    assert_eq!(save_preflight(&d, false), SavePreflight::Save);
    d.disk_changed = true;
    assert!(matches!(
        save_preflight(&d, false),
        SavePreflight::ReviewChanged(_)
    ));
    assert!(matches!(save_preflight(&d, true), SavePreflight::Picker(_)));
    d.copy = true;
    assert_eq!(
        save_preflight(&d, false),
        SavePreflight::Picker(Picker::save("Film", true))
    );
    d.saving = true;
    assert_eq!(save_preflight(&d, false), SavePreflight::Join(Work::Saving));
}
#[test]
fn passive_viewer_notice_expires_at_four_seconds_and_announces_once() {
    let mut s = NoticeSurface::new(
        handle("a"),
        reloaded("Film"),
        NoticeLocation::Viewer,
        Duration::ZERO,
    );
    assert_eq!(
        s.take_announcement(),
        Some((
            "“Film” was updated from another app.",
            AnnouncementPriority::Medium
        ))
    );
    assert!(s.take_announcement().is_none());
    assert!(!s.tick(Duration::from_millis(3999)));
    assert!(s.tick(Duration::from_secs(4)));
    assert!(!s.tick(Duration::from_secs(5)));
}
#[test]
fn action_banner_persists_until_action_or_dismiss_and_rejects_stale_generation() {
    let mut s = NoticeSurface::new(
        handle("a"),
        recovery_copy("Film", "recovery-a".into()),
        NoticeLocation::Viewer,
        Duration::ZERO,
    );
    assert!(!s.tick(Duration::from_secs(100)));
    assert!(s.visible);
    assert!(s.activate(&handle("b"), Action::ShowInFinder).is_none());
    assert!(s.activate(&handle("a"), Action::Save).is_none());
    let a = s.activate(&handle("a"), Action::ShowInFinder).unwrap();
    assert_eq!(a.recovery_id.as_deref(), Some("recovery-a"));
    assert!(!s.visible);
    assert!(s.activate(&handle("a"), Action::ShowInFinder).is_none());
    let mut s = NoticeSurface::new(
        handle("a"),
        uncertain_save("Film"),
        NoticeLocation::Viewer,
        Duration::ZERO,
    );
    assert_eq!(s.take_announcement().unwrap().1, AnnouncementPriority::High);
    assert!(s.activate(&handle("a"), Action::Dismiss).is_some());
}
#[test]
fn library_notice_does_not_acquire_viewer_timeout_or_dismiss_control() {
    let mut s = NoticeSurface::new(
        handle("a"),
        reloaded("Film"),
        NoticeLocation::Library,
        Duration::ZERO,
    );
    assert!(!s.tick(Duration::from_secs(100)));
    assert!(s.activate(&handle("a"), Action::Dismiss).is_none());
    assert!(!s.document_closed(&handle("b")));
    assert!(s.visible);
    assert!(s.document_closed(&handle("a")));
    assert!(s.take_announcement().is_none());
}
#[test]
fn passive_banner_has_no_implicit_action() {
    let mut s = NoticeSurface::new(
        handle("a"),
        reloaded("Film"),
        NoticeLocation::Viewer,
        Duration::ZERO,
    );
    assert!(s.activate(&handle("a"), Action::Dismiss).is_none());
    assert!(s.visible);
}
