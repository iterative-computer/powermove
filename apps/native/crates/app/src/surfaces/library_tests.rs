use super::{controller::*, form::*, kernel, model::*};
use crate::alert::{Alert, Key};
use pm_services::ServiceErrorCode;
use std::{
    collections::{BTreeSet, VecDeque},
    time::Duration,
};

#[derive(Default)]
struct Port {
    sent: Vec<Request>,
    completions: VecDeque<Completion>,
    refuse: bool,
}
impl LibraryPort for Port {
    fn enqueue(&mut self, request: Request) -> Result<(), Failure> {
        if self.refuse {
            return Err(Failure::Service(ServiceErrorCode::Unavailable));
        }
        self.sent.push(request);
        Ok(())
    }
    fn poll(&mut self) -> Option<Completion> {
        self.completions.pop_front()
    }
}
fn row(id: &str, name: &str, trashed: bool) -> Row {
    Row {
        id: id.into(),
        name: name.into(),
        modified_ms: Some(42),
        deleted_ms: None,
        trashed,
        file: None,
        edited: false,
        open: trashed,
        summary: Some(Summary {
            thumbnail: None,
            duration_seconds: Some(10.0),
            width: Some(1920),
            height: Some(1080),
            fps: Some(30.0),
        }),
    }
}
fn sample() -> Snapshot {
    Snapshot {
        rows: vec![
            row("a", "Launch Film", false),
            row("b", "Trailer", false),
            row("c", "Unused", true),
        ],
        recents: BTreeSet::from(["b".into()]),
    }
}
fn complete(
    controller: &mut LibraryController<Port>,
    serial: u64,
    result: Result<Outcome, Failure>,
) {
    controller
        .port
        .completions
        .push_back(Completion { serial, result });
    controller.poll();
}
fn load(controller: &mut LibraryController<Port>, snapshot: Snapshot) {
    let request = controller.port.sent.last().unwrap().clone();
    let Command::Load(query) = request.command else {
        panic!("expected load");
    };
    let rows = query.apply(&snapshot);
    complete(
        controller,
        request.serial,
        Ok(Outcome::Loaded { snapshot, rows }),
    );
}
fn ready() -> LibraryController<Port> {
    let mut c = LibraryController::new(Port::default());
    c.query.section = Section::All;
    c.refresh().unwrap();
    load(&mut c, sample());
    c
}
fn effects(c: &mut LibraryController<Port>) -> Vec<HostEffect> {
    c.take_effects()
}
fn alert(c: &mut LibraryController<Port>) -> (u64, Alert<bool>) {
    effects(c)
        .into_iter()
        .find_map(|effect| {
            if let HostEffect::Alert { serial, alert, .. } = effect {
                Some((serial, alert))
            } else {
                None
            }
        })
        .unwrap()
}
fn format() -> Format {
    Format {
        width: 1920,
        height: 1080,
        fps: 30.0,
        duration: 10.0,
        background: "#09090A".into(),
    }
}

#[test]
fn all_empty_states_use_v1_home_copy() {
    assert_eq!(
        empty_state(&Query::default()),
        EmptyState {
            title: "No projects yet",
            symbol: "folder",
            detail:
                "Projects keep the composition, layers, reusable sections and exports together."
                    .into(),
            action: Some("Create a project")
        }
    );
    assert_eq!(
        empty_state(&Query {
            section: Section::Deleted,
            ..Default::default()
        }),
        EmptyState {
            title: "Trash is empty",
            symbol: "trash",
            detail:
                "Projects moved to Trash stay here until you restore or permanently delete them."
                    .into(),
            action: None
        }
    );
    assert_eq!(
        empty_state(&Query {
            text: "Launch".into(),
            ..Default::default()
        }),
        EmptyState {
            title: "No matching projects",
            symbol: "magnifyingglass",
            detail: "Try a different name.".into(),
            action: None
        }
    );
}

#[test]
fn loading_then_storage_retry_is_available() {
    let mut c = LibraryController::new(Port::default());
    c.refresh().unwrap();
    assert_eq!(c.content(), Content::Loading);
    let serial = c.port.sent[0].serial;
    complete(
        &mut c,
        serial,
        Err(Failure::Service(ServiceErrorCode::DiskFull)),
    );
    assert_eq!(
        c.content(),
        Content::Unavailable {
            message: "Your projects can’t be shown right now.",
            action: "Try Again"
        }
    );
    c.refresh().unwrap();
    load(&mut c, Snapshot::default());
    assert!(matches!(c.content(), Content::Empty(_)));
}

#[test]
fn worker_search_retains_previous_rows_and_discards_out_of_order_completions() {
    let mut c = ready();
    c.set_query(Query {
        text: "Launch".into(),
        ..Default::default()
    })
    .unwrap();
    let stale = c.port.sent.last().unwrap().serial;
    c.set_query(Query {
        text: "Trailer".into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(c.rows.len(), 2);
    complete(
        &mut c,
        stale,
        Ok(Outcome::Loaded {
            snapshot: Snapshot::default(),
            rows: vec![],
        }),
    );
    assert_eq!(c.rows.len(), 2);
    load(&mut c, sample());
    assert_eq!(c.rows[0].id, "b");
}

#[test]
fn context_menus_match_spec_and_file_backing_controls_reveal() {
    let mut r = row("a", "Film", false);
    let labels = |r: &Row| {
        context_menu(r)
            .into_iter()
            .map(|a| a.map(MenuAction::label))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(&r),
        vec![
            Some("Open"),
            Some("Open in New Window"),
            None,
            Some("Rename…"),
            Some("Duplicate"),
            None,
            Some("Move to Trash…")
        ]
    );
    r.file = Some(std::env::temp_dir().join("Film.pmv"));
    assert!(labels(&r).contains(&Some("Show in Finder")));
    r.trashed = true;
    assert_eq!(
        labels(&r),
        vec![Some("Restore"), None, Some("Delete Forever…")]
    );
}

#[test]
fn vanished_rename_target_cannot_receive_a_commit() {
    let mut c = ready();
    c.menu("a", MenuAction::Rename).unwrap();
    c.refresh().unwrap();
    load(&mut c, Snapshot::default());
    assert!(c.rename.is_none());
    assert_eq!(c.menu("a", MenuAction::Open), Err(Failure::Stale));
}

#[test]
fn duplicate_uniques_name_then_selects_and_starts_rename_after_ack() {
    let mut c = ready();
    c.snapshot.rows.push(row("copy", "Launch Film copy", false));
    c.menu("a", MenuAction::Duplicate).unwrap();
    let request = c.port.sent.last().unwrap().clone();
    assert_eq!(
        request.command,
        Command::Duplicate {
            id: "a".into(),
            name: "Launch Film copy 2".into()
        }
    );
    assert!(c.rename.is_none());
    complete(&mut c, request.serial, Ok(Outcome::Duplicated("d".into())));
    let mut snapshot = sample();
    snapshot.rows.push(row("d", "Launch Film copy 2", false));
    load(&mut c, snapshot);
    assert_eq!(c.selected.as_deref(), Some("d"));
    assert_eq!(c.rename.as_ref().unwrap().id, "d");
}

#[test]
fn put_back_has_no_confirmation_and_does_not_open_a_project() {
    let mut c = ready();
    c.menu("c", MenuAction::PutBack).unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::PutBack("c".into())
    );
    assert!(
        !effects(&mut c)
            .iter()
            .any(|e| matches!(e, HostEffect::Alert { .. }))
    );
}

#[test]
fn delete_is_critical_archival_copy_and_cancel_is_default() {
    let mut c = ready();
    c.menu("c", MenuAction::DeleteImmediately).unwrap();
    let (serial, prompt) = alert(&mut c);
    assert_eq!(prompt.message, "Delete “Unused” forever?");
    assert_eq!(
        prompt.informative,
        "The project is archived in Powermove’s storage and can’t be opened from Home again."
    );
    assert_eq!(prompt.labels(), ["Cancel", "Delete Forever"]);
    assert!(prompt.buttons[0].default);
    assert_eq!(prompt.buttons[0].key, Key::Escape);
    assert!(prompt.buttons[1].destructive && !prompt.buttons[0].destructive);
    assert!(prompt.critical);
    let count = c.port.sent.len();
    c.answer(serial, None).unwrap();
    assert_eq!(c.port.sent.len(), count);
}

#[test]
fn stale_alert_response_and_abort_never_mutate_the_wrong_target() {
    let mut c = ready();
    c.menu("a", MenuAction::Trash).unwrap();
    let (serial, _) = alert(&mut c);
    assert_eq!(c.answer(serial + 1, Some(true)), Err(Failure::Stale));
    c.answer(serial, None).unwrap();
    assert!(
        c.port
            .sent
            .iter()
            .all(|r| !matches!(r.command, Command::Trash(_)))
    );
    assert_eq!(c.answer(serial, Some(true)), Err(Failure::Stale));
}

#[test]
fn alerts_queue_in_arrival_order() {
    let mut c = ready();
    c.menu("a", MenuAction::Trash).unwrap();
    c.menu("b", MenuAction::Trash).unwrap();
    let (first, prompt) = alert(&mut c);
    assert!(prompt.message.contains("Launch Film"));
    assert!(
        !effects(&mut c)
            .iter()
            .any(|e| matches!(e, HostEffect::Alert { .. }))
    );
    c.answer(first, None).unwrap();
    let (second, prompt) = alert(&mut c);
    assert!(prompt.message.contains("Trailer"));
    assert!(second > first);
}

#[test]
fn new_project_defaults_current_format_and_uniqued_untitled_name() {
    let current = Format {
        width: 1080,
        height: 1920,
        fps: 30000.0 / 1001.0,
        duration: 12.5,
        background: "#12345678".into(),
    };
    let f = NewProjectForm::new(
        "owner".into(),
        Some(&current),
        &format(),
        &["Untitled".into(), "Untitled 2".into()],
    );
    assert_eq!(f.name, "Untitled 3");
    assert_eq!(f.values().unwrap().format, current);
    assert_eq!(f.linked, Some(true));
    let f = NewProjectForm::new("owner".into(), None, &format(), &[]);
    assert_eq!(f.name, "Untitled");
    assert_eq!(f.values().unwrap().format, format());
}

#[test]
fn preset_copy_dimensions_and_manual_edits_switch_to_custom() {
    assert_eq!(
        pm_builtins::RESOLUTIONS.map(|p| p.0),
        [
            "4K UHD · 3840×2160",
            "HD 1080p · 1920×1080",
            "HD 720p · 1280×720",
            "Vertical HD · 1080×1920",
            "Portrait 4:5 · 1080×1350",
            "Square · 1080×1080"
        ]
    );
    let mut f = NewProjectForm::new("owner".into(), None, &format(), &[]);
    for (index, (_, w, h)) in pm_builtins::RESOLUTIONS.iter().enumerate() {
        f.choose_preset(Some(index)).unwrap();
        assert_eq!(f.width, w.to_string());
        assert_eq!(f.height, h.to_string());
    }
    f.edit_size("1920".into(), "1080".into()).unwrap();
    assert_eq!(f.preset, None);
}

#[test]
fn live_validation_preserves_drafts_and_exact_error_strings() {
    let mut f = NewProjectForm::new("owner".into(), None, &format(), &[]);
    f.name = "  ".into();
    f.width = "15".into();
    f.height = "8193".into();
    f.fps = "NaN".into();
    let v = f.validation();
    assert_eq!(v.name, Some("Enter a name."));
    assert_eq!(v.size, Some("Enter a size from 16 to 8192 pixels."));
    assert_eq!(v.fps, Some("Enter a frame rate from 1 to 240 fps."));
    assert!(!f.can_create());
    f.name = "Film".into();
    f.width = "16".into();
    f.height = "8192".into();
    f.fps = "240".into();
    f.duration = "3600.1".into();
    assert_eq!(
        f.validation().duration,
        Some("Enter a duration up to 1 hour.")
    );
    f.duration = "0.1".into();
    assert!(f.can_create());
    assert_eq!(f.values().unwrap().format.width, 16);
    assert_eq!(f.height, "8192");
}

#[test]
fn invalid_numbers_never_clamp_or_reach_the_service() {
    for (fps, duration, width) in [
        ("0", "10", "1920"),
        ("241", "10", "1920"),
        ("inf", "10", "1920"),
        ("30", "0.09", "1920"),
        ("30", "3601", "1920"),
        ("30", "10junk", "1920"),
        ("30", "10", "100.5"),
    ] {
        let mut c = ready();
        c.begin_new("owner".into(), None, &format()).unwrap();
        let f = c.form.as_mut().unwrap();
        f.fps = fps.into();
        f.duration = duration.into();
        f.width = width.into();
        let count = c.port.sent.len();
        assert!(c.create().is_err());
        assert_eq!(c.port.sent.len(), count);
        assert!(c.form.is_some());
    }
}

#[test]
fn timecode_preserves_exact_rate_and_rejects_drop_frame_and_bad_fields() {
    assert_eq!(parse_duration("00:01:02:15", 30.0), Some(62.5));
    assert!((parse_duration("00:01:00:00", 30000.0 / 1001.0).unwrap() - 60.06).abs() < 1e-10);
    for value in [
        "00:00:00:30",
        "00:60:00:00",
        "00:00:60:00",
        "00:00:00;02",
        "1:2",
        "-1:00:00:00",
        "00::00:00",
    ] {
        assert_eq!(parse_duration(value, 30.0), None);
    }
}

#[test]
fn create_failure_keeps_sheet_and_draft_then_retry_closes_after_success() {
    let mut c = ready();
    c.begin_new("owner".into(), None, &format()).unwrap();
    c.form.as_mut().unwrap().name = "Launch Film".into();
    c.create().unwrap();
    assert!(c.form.as_ref().unwrap().submitting);
    assert_eq!(c.create(), Err(Failure::Busy));
    let serial = c.port.sent.last().unwrap().serial;
    complete(
        &mut c,
        serial,
        Err(Failure::Service(ServiceErrorCode::DiskFull)),
    );
    assert_eq!(c.form.as_ref().unwrap().name, "Launch Film");
    assert!(!c.form.as_ref().unwrap().submitting);
    let (serial, prompt) = alert(&mut c);
    assert_eq!(prompt.message, "There isn’t enough disk space.");
    c.answer(serial, Some(false)).unwrap();
    c.create().unwrap();
    let serial = c.port.sent.last().unwrap().serial;
    complete(&mut c, serial, Ok(Outcome::Changed));
    assert!(c.form.is_none());
    assert!(
        effects(&mut c)
            .iter()
            .any(|e| matches!(e, HostEffect::CloseNewProject))
    );
}

#[test]
fn service_errors_use_spec_sentences_and_never_show_diagnostics() {
    let cases = [
        (
            ServiceErrorCode::Offline,
            "Powermove can’t connect.",
            "Check your internet connection and try again.",
        ),
        (
            ServiceErrorCode::NotFound,
            "“Launch Film” can’t be found.",
            "It may have been moved or deleted.",
        ),
        (
            ServiceErrorCode::TooLarge,
            "“Launch Film” is too large to open.",
            "",
        ),
        (
            ServiceErrorCode::InvalidProject,
            "“Launch Film” can’t be opened because it isn’t a Powermove project.",
            "",
        ),
        (
            ServiceErrorCode::PermissionDenied,
            "You don’t have permission to open “Launch Film”.",
            "Check its sharing and permissions in Finder.",
        ),
        (
            ServiceErrorCode::DiskFull,
            "There isn’t enough disk space.",
            "Free some space and try again.",
        ),
        (
            ServiceErrorCode::InvalidInput,
            "“Launch Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            ServiceErrorCode::InvalidData,
            "“Launch Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            ServiceErrorCode::Unavailable,
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            ServiceErrorCode::Unknown,
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    ];
    for (code, message, detail) in cases {
        let alert = error_alert(code, "Launch Film");
        assert_eq!(alert.message, message);
        assert_eq!(alert.informative, detail);
        assert_eq!(alert.labels(), ["OK"]);
        assert!(!alert.critical);
    }
}

#[test]
fn disconnected_service_and_unsupported_operations_are_not_successes() {
    let mut c = LibraryController::new(Port {
        refuse: true,
        ..Default::default()
    });
    assert!(c.refresh().is_err());
    assert!(matches!(c.content(), Content::Unavailable { .. }));
    let mut c = ready();
    c.menu("c", MenuAction::DeleteImmediately).unwrap();
    let (serial, _) = alert(&mut c);
    c.answer(serial, Some(true)).unwrap();
    let serial = c.port.sent.last().unwrap().serial;
    complete(
        &mut c,
        serial,
        Err(Failure::Unsupported("archive API missing")),
    );
    assert!(c.snapshot.rows.iter().any(|r| r.id == "c"));
    assert!(
        effects(&mut c)
            .iter()
            .any(|e| matches!(e, HostEffect::IntegrationGap("archive API missing")))
    );
}

fn service() -> (
    tempfile::TempDir,
    pm_kernel::KernelActor,
    kernel::KernelLibrary,
) {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().canonicalize().unwrap();
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
    let reply = actor
        .handle()
        .shell(pm_kernel::session::ShellCall::AttachLibrary {
            profile: profile.clone(),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        reply,
        pm_kernel::session::ShellReply::Library(pm_kernel::locks::OpenDecision::Writable)
    ));
    let service = kernel::KernelLibrary {
        kernel: actor.handle(),
        profile,
    };
    (dir, actor, service)
}
fn create_real(service: &mut kernel::KernelLibrary) -> pm_kernel::session::SessionHandle {
    let Outcome::Opened(pm_kernel::session::ShellReply::Opened { session, .. }) = service
        .execute(Command::Create(NewProject {
            name: "Launch Film".into(),
            format: format(),
        }))
        .unwrap()
    else {
        panic!("open");
    };
    session
}

#[test]
fn real_kernel_creates_renames_duplicates_and_focuses_existing_session() {
    let (_dir, actor, mut service) = service();
    let session = create_real(&mut service);
    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.rows[0].name, "Launch Film");
    assert_eq!(snapshot.rows[0].summary.as_ref().unwrap().fps, Some(30.0));
    let outcome = service
        .execute(Command::Open(session.project.clone()))
        .unwrap();
    assert!(matches!(outcome,Outcome::Focus(id) if id == session.project));
    service
        .execute(Command::Rename {
            id: session.project.clone(),
            name: "Renamed Film".into(),
        })
        .unwrap();
    assert_eq!(service.snapshot().unwrap().rows[0].name, "Renamed Film");
    let Outcome::Duplicated(copy) = service
        .execute(Command::Duplicate {
            id: session.project.clone(),
            name: "Renamed Film copy".into(),
        })
        .unwrap()
    else {
        panic!("duplicate");
    };
    assert_ne!(copy, session.project);
    assert_eq!(service.snapshot().unwrap().rows.len(), 2);
    actor.shutdown();
}

#[test]
fn real_kernel_trash_restore_keeps_document_bytes_and_archive_refuses_safely() {
    let (_dir, actor, mut service) = service();
    let session = create_real(&mut service);
    assert!(matches!(
        service.execute(Command::Trash(session.project.clone())),
        Err(Failure::Unsupported(_))
    ));
    service
        .kernel
        .shell(pm_kernel::session::ShellCall::Close(session.clone()))
        .wait()
        .unwrap()
        .unwrap();
    let slot = service
        .profile
        .join("store")
        .join(format!("project.{}.json", session.project));
    let bytes = std::fs::read(&slot).unwrap();
    service
        .execute(Command::Trash(session.project.clone()))
        .unwrap();
    assert!(service.snapshot().unwrap().rows[0].trashed);
    assert!(matches!(
        service.execute(Command::Archive(session.project.clone())),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(std::fs::read(&slot).unwrap(), bytes);
    assert!(service.snapshot().unwrap().rows[0].trashed);
    service
        .execute(Command::PutBack(session.project.clone()))
        .unwrap();
    assert!(!service.snapshot().unwrap().rows[0].trashed);
    assert_eq!(std::fs::read(&slot).unwrap(), bytes);
    actor.shutdown();
}

#[test]
fn real_kernel_closed_slot_rename_is_explicitly_unavailable_and_does_not_open_it() {
    let (_dir, actor, mut service) = service();
    let session = create_real(&mut service);
    service
        .kernel
        .shell(pm_kernel::session::ShellCall::Close(session.clone()))
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        service.execute(Command::Rename {
            id: session.project.clone(),
            name: "Other".into()
        }),
        Err(Failure::Unsupported(_))
    ));
    assert!(service.kernel.published(&session.project).is_none());
    assert_eq!(service.snapshot().unwrap().rows[0].name, "Launch Film");
    actor.shutdown();
}

#[test]
fn service_boundary_validates_inputs_without_creating_slots() {
    let (_dir, actor, mut service) = service();
    let invalid = NewProject {
        name: "Film".into(),
        format: Format {
            fps: f64::NAN,
            ..format()
        },
    };
    assert!(service.execute(Command::Create(invalid)).is_err());
    for id in ["../other", "", "a/b"] {
        assert!(service.execute(Command::Trash(id.into())).is_err());
    }
    assert!(service.snapshot().unwrap().rows.is_empty());
    actor.shutdown();
}

#[test]
fn worker_port_returns_before_io_and_wakes_with_id_correlated_completion() {
    let (_dir, actor, service) = service();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut port = kernel::KernelLibraryPort::start(
        service.kernel,
        service.profile,
        std::sync::Arc::new(move || {
            let _ = tx.send(());
        }),
    )
    .unwrap();
    port.enqueue(Request {
        serial: 99,
        command: Command::Load(Query::default()),
    })
    .unwrap();
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let completion = port.poll().unwrap();
    assert_eq!(completion.serial, 99);
    assert!(matches!(completion.result, Ok(Outcome::Loaded { .. })));
    drop(port);
    actor.shutdown();
}

#[test]
fn recents_default_and_twelve_item_limit_preserve_tagged_authored_behavior() {
    let c = LibraryController::new(Port::default());
    assert_eq!(c.query.section, Section::Recents);
    let snapshot = Snapshot {
        rows: (0..20)
            .map(|i| row(&format!("P{i}"), "Film", false))
            .collect(),
        recents: (0..20).map(|i| format!("P{i}")).collect(),
    };
    let query = Query {
        section: Section::Recents,
        ..Default::default()
    };
    assert_eq!(query.apply(&snapshot).len(), 12);
    assert_eq!(
        Query {
            text: " film ".into(),
            ..query
        }
        .apply(&snapshot)
        .len(),
        20
    );
}

#[test]
fn real_kernel_rejects_invalid_colour_and_preserves_fractional_rate_and_background() {
    let (_dir, actor, mut service) = service();
    let mut input = NewProject {
        name: "Exact".into(),
        format: Format {
            fps: 30000.0 / 1001.0,
            background: "#AaBbCcDD".into(),
            ..format()
        },
    };
    let Outcome::Opened(pm_kernel::session::ShellReply::Opened { session, .. }) =
        service.execute(Command::Create(input.clone())).unwrap()
    else {
        panic!("opened");
    };
    let published = service.kernel.published(&session.project).unwrap();
    let value =
        pm_model::canonical::realize_project(published.snapshot.document.to_value().unwrap());
    assert_eq!(
        value.get("fps").and_then(pm_model::Value::as_f64),
        Some(30000.0 / 1001.0)
    );
    assert_eq!(
        value.get("bg").and_then(pm_model::Value::as_str),
        Some("#AaBbCcDD")
    );
    input.format.background = "invalid".into();
    assert!(service.execute(Command::Create(input)).is_err());
    assert_eq!(service.snapshot().unwrap().rows.len(), 1);
    actor.shutdown();
}

#[test]
fn real_kernel_open_hands_a_closed_project_to_the_editor() {
    let (_dir, actor, mut service) = service();
    let session = create_real(&mut service);
    service
        .kernel
        .shell(pm_kernel::session::ShellCall::Close(session.clone()))
        .wait()
        .unwrap()
        .unwrap();
    let result = service
        .execute(Command::Open(session.project.clone()))
        .unwrap();
    let Outcome::Opened(pm_kernel::session::ShellReply::Opened {
        session: opened, ..
    }) = result
    else {
        panic!("opened");
    };
    assert_eq!(opened.project, session.project);
    assert_ne!(opened.generation, session.generation);
    actor.shutdown();
}

#[test]
fn reviewed_timecode_rejects_skipped_labels_and_non_drop_rates() {
    for (fps, drop) in [
        (29.97, 2_u32),
        (59.94, 4),
        (30000. / 1001., 2),
        (60000. / 1001., 4),
    ] {
        assert_eq!(parse_duration("00:01:00;00", fps), None);
        assert_eq!(
            parse_duration(&format!("00:01:00;{drop:02}"), fps),
            Some(f64::from(fps.ceil() as u32 * 60) / fps)
        );
        assert_eq!(
            parse_duration("00:10:00;00", fps),
            Some((fps.ceil() * 600. - f64::from(drop) * 9.) / fps)
        );
    }
    let mut f = NewProjectForm::new("w".into(), None, &format(), &[]);
    f.fps = "24".into();
    f.duration = "00:01:00;00".into();
    assert_eq!(f.validation().duration, Some(DROP_FRAME_ERROR));
    assert!(!f.can_create());
    f.duration = "00:01:00:00".into();
    assert!(f.can_create());
}
#[test]
fn reviewed_link_and_admitted_creation_have_exact_controls() {
    let mut f = NewProjectForm::new("w".into(), None, &format(), &[]);
    assert_eq!(f.linked, Some(true));
    assert_eq!(
        f.link_presentation(),
        ("link", "Lock aspect ratio", "Unlock Aspect Ratio")
    );
    f.edit_dimension(true, "960".into()).unwrap();
    assert_eq!(f.height, "540");
    f.linked = Some(false);
    f.edit_dimension(true, "1920".into()).unwrap();
    assert_eq!(f.height, "540");
    f.submitting = true;
    assert_eq!(f.buttons(), ("Creating…", false, false));
    assert_eq!(f.edit_dimension(false, "1080".into()), Err(Failure::Busy));
    f.submitting = false;
    assert_eq!(f.buttons(), ("Create", true, true));
}
#[test]
fn reviewed_duplicate_clears_only_nonmatching_search_before_select_and_rename() {
    let mut c = ready();
    c.query.text = "Film".into();
    c.menu("a", MenuAction::Duplicate).unwrap();
    let serial = c.port.sent.last().unwrap().serial;
    complete(&mut c, serial, Ok(Outcome::Duplicated("copy".into())));
    let serial = c.port.sent.last().unwrap().serial;
    let mut snapshot = c.snapshot.clone();
    snapshot.rows.push(row("copy", "Other copy", false));
    complete(
        &mut c,
        serial,
        Ok(Outcome::Loaded {
            snapshot: snapshot.clone(),
            rows: vec![],
        }),
    );
    assert!(c.query.text.is_empty());
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::Load(c.query.clone())
    );
    let serial = c.port.sent.last().unwrap().serial;
    complete(
        &mut c,
        serial,
        Ok(Outcome::Loaded {
            snapshot,
            rows: vec![row("copy", "Other copy", false)],
        }),
    );
    assert_eq!(c.selected.as_deref(), Some("copy"));
    assert_eq!(c.rename.as_ref().unwrap().id, "copy");
}
#[test]
fn filter_uses_service_recents_and_does_not_mix_trash() {
    let snapshot = sample();
    assert_eq!(
        Query {
            section: Section::Recents,
            ..Default::default()
        }
        .apply(&snapshot)
        .iter()
        .map(|r| &r.id)
        .collect::<Vec<_>>(),
        vec!["b"]
    );
    assert_eq!(
        Query {
            section: Section::Deleted,
            ..Default::default()
        }
        .apply(&snapshot)[0]
            .id,
        "c"
    );
    assert_eq!(
        Query {
            text: "LAUNCH".into(),
            ..Default::default()
        }
        .apply(&snapshot)
        .len(),
        1
    );
    let mut snapshot = snapshot;
    snapshot.rows[0].modified_ms = Some(10);
    snapshot.rows[1].modified_ms = Some(20);
    assert_eq!(Query::default().apply(&snapshot)[0].id, "b");
}
#[test]
fn rename_escape_empty_and_unchanged_match_finder_behavior() {
    let mut c = ready();
    let count = c.port.sent.len();
    c.menu("a", MenuAction::Rename).unwrap();
    c.rename.as_mut().unwrap().text = "Changed".into();
    c.escape();
    assert!(c.rename.is_none());
    assert_eq!(c.port.sent.len(), count);
    for value in ["", "   ", "Launch Film"] {
        c.menu("a", MenuAction::Rename).unwrap();
        c.rename.as_mut().unwrap().text = value.into();
        c.commit_rename().unwrap();
        assert_eq!(c.port.sent.len(), count);
    }
    c.menu("a", MenuAction::Rename).unwrap();
    c.rename.as_mut().unwrap().text = "  New Film  ".into();
    c.commit_rename().unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::Rename {
            id: "a".into(),
            name: "New Film".into()
        }
    );
}
#[test]
fn double_click_opens_by_id() {
    for option in [false, true] {
        let mut c = ready();
        c.double_click("b", option).unwrap();
        assert_eq!(
            c.port.sent.last().unwrap().command,
            Command::Open("b".into())
        );
    }
}
#[test]
fn trash_requires_confirmation() {
    let mut c = ready();
    let count = c.port.sent.len();
    c.menu("a", MenuAction::Trash).unwrap();
    let (serial, prompt) = alert(&mut c);
    assert_eq!(prompt.message, "Move “Launch Film” to Trash?");
    assert_eq!(
        prompt.informative,
        "You can restore this project from Trash."
    );
    assert_eq!(prompt.labels(), ["Move to Trash", "Cancel"]);
    assert!(prompt.buttons[0].default);
    assert_eq!(prompt.buttons[1].key, Key::Escape);
    assert!(!prompt.critical);
    assert_eq!(c.port.sent.len(), count);
    c.answer(serial, Some(true)).unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::Trash("a".into())
    );
}
#[test]
fn card_facts_read_as_v1_date_file_and_size() {
    const MIN: i64 = 60_000;
    let now = 1_000 * MIN;
    let mut r = row("a", "Launch Film", false);
    r.modified_ms = Some(now - 3 * MIN);
    assert_eq!(r.facts(now), "Edited 3 min ago · 1920×1080");
    r.file = Some(std::path::PathBuf::from("/tmp/Launch Film.pmv"));
    assert_eq!(
        r.facts(now),
        "Edited 3 min ago · Launch Film.pmv · 1920×1080"
    );
    r.edited = true;
    assert_eq!(
        r.facts(now),
        "Edited 3 min ago · Unsaved changes · Launch Film.pmv · 1920×1080"
    );
    r.summary.as_mut().unwrap().width = Some(0);
    r.file = None;
    r.modified_ms = Some(now - 1_000);
    assert_eq!(r.facts(now), "Edited just now");
    r.trashed = true;
    r.deleted_ms = Some(now - 2 * 86_400_000);
    assert_eq!(r.facts(now), "Deleted 2 d ago");
    // Past a week, the date (v1 shipped en-US `toLocaleDateString`).
    assert_eq!(
        ago(Some(1_767_225_600_000), 1_767_225_600_000 + 30 * 86_400_000),
        "1/1/2026"
    );
    assert_eq!(ago(Some(now - 5 * 3_600_000), now), "5 h ago");
}
#[test]
fn sort_by_name_ignores_case_and_recent_puts_newest_first() {
    let mut a = row("a", "beta", false);
    a.modified_ms = Some(1);
    let mut b = row("b", "Alpha", false);
    b.modified_ms = Some(2);
    let snapshot = Snapshot {
        rows: vec![a, b],
        recents: ["a".into(), "b".into()].into(),
    };
    let names = |sort| {
        Query {
            section: Section::All,
            sort,
            ..Default::default()
        }
        .apply(&snapshot)
        .into_iter()
        .map(|r| r.name)
        .collect::<Vec<_>>()
    };
    assert_eq!(names(Sort::Recent), ["Alpha", "beta"]);
    assert_eq!(names(Sort::Name), ["Alpha", "beta"]);
    let mut c = snapshot.clone();
    c.rows[1].modified_ms = Some(0);
    let recent: Vec<_> = Query {
        section: Section::All,
        ..Default::default()
    }
    .apply(&c)
    .into_iter()
    .map(|r| r.name)
    .collect();
    assert_eq!(recent, ["beta", "Alpha"]);
    assert_eq!(Sort::from_key(Sort::Name.key()), Sort::Name);
    assert_eq!(Section::from_key("trash"), Section::Deleted);
    assert_eq!(Section::from_key("projects"), Section::Recents);
    assert_eq!(Layout::from_key("list"), Layout::List);
}
#[test]
fn open_in_new_window_asks_for_a_window_of_its_own() {
    use pm_kernel::{locks::OpenDecision, session::ShellReply};
    for (action, new_window) in [(MenuAction::Open, false), (MenuAction::OpenNewWindow, true)] {
        let mut c = ready();
        c.menu("b", action).unwrap();
        let serial = c.port.sent.last().unwrap().serial;
        c.take_effects();
        let reply = ShellReply::Library(OpenDecision::Writable);
        complete(&mut c, serial, Ok(Outcome::Opened(reply)));
        let opened = c.take_effects().into_iter().find_map(|e| match e {
            HostEffect::OpenEditor {
                name, new_window, ..
            } => Some((name, new_window)),
            _ => None,
        });
        assert_eq!(opened, Some(("Trailer".into(), new_window)));
    }
}
