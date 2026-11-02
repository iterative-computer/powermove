use super::*;
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
            return Err(Failure::Service(ErrorCode::Unavailable));
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
    c.show().unwrap();
    load(&mut c, sample());
    c
}
fn metrics() -> pm_design::Tokens {
    pm_design::Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default())
}
fn effects(c: &mut LibraryController<Port>) -> Vec<HostEffect> {
    let mut host = HeadlessHost::default();
    c.render(&mut host, &metrics(), Duration::ZERO, 0..10);
    host.effects
}
fn alert(c: &mut LibraryController<Port>) -> (u64, Alert) {
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
fn window_contract_uses_exact_spec_copy_and_native_pieces() {
    assert_eq!(TITLE, "Project Library");
    assert_eq!(SEARCH, "Search Projects");
    assert_eq!(
        SIDEBAR,
        &[
            ("Recents", "clock"),
            ("All Projects", "square.grid.2x2"),
            ("Recently Deleted", "trash")
        ]
    );
    assert_eq!(
        VIEW_BUTTONS,
        &[
            ("Grid View", "square.grid.2x2"),
            ("List View", "list.bullet")
        ]
    );
    assert_eq!(SORT_OPTIONS, &["Date Modified", "Name"]);
    assert_eq!(
        COLUMNS,
        &["Name", "Date Modified", "Duration", "Size", "Frame Rate"]
    );
    for native in [
        "NSToolbar",
        "NSSplitViewController",
        "NSTableView",
        "NSAlert.beginSheetModal",
        "NSWindow.beginSheet",
    ] {
        assert!(NATIVE_PIECES.contains(&native));
    }
}

#[test]
fn all_empty_states_and_quoted_search_copy() {
    assert_eq!(
        empty_state(&Query::default()),
        EmptyState {
            title: "No Projects",
            detail: "Projects you create appear here.".into(),
            action: Some("New Project")
        }
    );
    assert_eq!(
        empty_state(&Query {
            section: Section::Deleted,
            ..Default::default()
        }),
        EmptyState {
            title: "No Deleted Projects",
            detail: "Projects you move to the Trash appear here.".into(),
            action: None
        }
    );
    assert_eq!(
        empty_state(&Query {
            text: "Launch".into(),
            ..Default::default()
        }),
        EmptyState {
            title: "No Results",
            detail: "No projects match “Launch”.".into(),
            action: None
        }
    );
}

#[test]
fn loading_does_not_flash_and_storage_retry_is_available() {
    let mut c = LibraryController::new(Port::default());
    c.show().unwrap();
    assert_eq!(
        c.presentation(Duration::from_millis(499), 0..0).content,
        Content::Loading { spinner: false }
    );
    assert_eq!(
        c.presentation(Duration::from_millis(500), 0..0).content,
        Content::Loading { spinner: true }
    );
    let serial = c.port.sent[0].serial;
    complete(&mut c, serial, Err(Failure::Service(ErrorCode::DiskFull)));
    assert_eq!(
        c.presentation(Duration::ZERO, 0..0).content,
        Content::Unavailable {
            message: "Your projects can’t be shown right now.",
            action: "Try Again"
        }
    );
    c.refresh().unwrap();
    load(&mut c, Snapshot::default());
    assert!(matches!(
        c.presentation(Duration::ZERO, 0..0).content,
        Content::Empty(_)
    ));
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
    assert!(c.presentation(Duration::ZERO, 0..10).searching);
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
fn filter_sort_uses_service_recents_and_does_not_mix_trash() {
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
    assert_eq!(
        Query {
            sort: Sort::Name,
            ..Default::default()
        }
        .apply(&snapshot)[0]
            .id,
        "a"
    );
}

#[test]
fn large_library_only_publishes_viewport_with_overscan_and_preserves_id_anchor() {
    let rows = (0..60_000)
        .map(|i| row(&format!("P{i}"), "Film", false))
        .collect::<Vec<_>>();
    let snapshot = Snapshot {
        rows,
        recents: BTreeSet::new(),
    };
    let mut c = ready();
    c.refresh().unwrap();
    load(&mut c, snapshot);
    c.set_scroll_anchor(Some("P30000".into()));
    let range = virtual_range(60_000, 10_000, 10, 3);
    assert_eq!(range, 29_970..30_060);
    let p = c.presentation(Duration::ZERO, range);
    assert_eq!(p.total_rows, 60_000);
    assert_eq!(p.rows.len(), 90);
    assert_eq!(p.anchor, Some("P30000".into()));
    assert_eq!(
        virtual_range(10, usize::MAX, usize::MAX, usize::MAX),
        10..10
    );
}

#[test]
fn voiceover_and_card_facts_use_system_date_and_authored_dimensions() {
    let mut r = row("a", "Launch Film", false);
    assert_eq!(
        r.ax_label("Today, 11:42"),
        Some("Launch Film, edited Today, 11:42, 1920 by 1080".into())
    );
    assert_eq!(r.facts("Yesterday"), Some("Yesterday · 1920×1080".into()));
    assert_eq!(r.file_tooltip(), "Not saved to a file");
    r.file = Some(std::env::temp_dir().join("Launch.pmv"));
    assert_eq!(
        r.file_tooltip(),
        format!("Saved to {}", r.file.as_ref().unwrap().display())
    );
    assert_eq!(r.edited_label(), None);
    r.edited = true;
    assert_eq!(r.edited_label(), Some("Edited"));
    r.summary = None;
    assert_eq!(r.ax_label("Yesterday"), None);
    assert_eq!(r.facts("Yesterday"), None);
    let mut c = ready();
    assert_eq!(c.presentation(Duration::ZERO, 0..10).ax_container, "AXList");
    assert_eq!(c.presentation(Duration::ZERO, 0..10).ax_item, "AXGroup");
    c.view = View::List;
    assert_eq!(
        c.presentation(Duration::ZERO, 0..10).ax_container,
        "AXTable"
    );
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
            Some("Move to Trash")
        ]
    );
    r.file = Some(std::env::temp_dir().join("Film.pmv"));
    assert!(labels(&r).contains(&Some("Show in Finder")));
    r.trashed = true;
    assert_eq!(
        labels(&r),
        vec![Some("Put Back"), None, Some("Delete Immediately…")]
    );
}

#[test]
fn keyboard_selection_obeys_grid_columns_list_rows_and_native_type_select() {
    let mut c = ready();
    c.key(Key::Right, 3).unwrap();
    assert_eq!(c.selected.as_deref(), Some("a"));
    c.key(Key::Down, 3).unwrap();
    assert_eq!(c.selected.as_deref(), Some("b"));
    c.key(Key::Up, 3).unwrap();
    assert_eq!(c.selected.as_deref(), Some("a"));
    c.view = View::List;
    c.key(Key::Down, 100).unwrap();
    assert_eq!(c.selected.as_deref(), Some("b"));
    c.type_select("launch");
    assert_eq!(c.selected.as_deref(), Some("a"));
    c.key(Key::Find, 1).unwrap();
    assert!(
        effects(&mut c)
            .iter()
            .any(|e| matches!(e, HostEffect::FocusSearch))
    );
    assert_eq!(
        KEYBOARD
            .iter()
            .filter(|(_, k)| *k == Key::NewProject)
            .map(|(s, _)| *s)
            .collect::<Vec<_>>(),
        vec!["⌥⌘N"]
    );
    assert!(KEYBOARD.iter().all(|(s, _)| *s != "⇧⌘O"));
}

#[test]
fn return_and_option_double_click_preserve_open_intent_by_id() {
    let mut c = ready();
    c.select("a").unwrap();
    c.key(Key::Return, 1).unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::Open {
            id: "a".into(),
            new_window: false
        }
    );
    let mut c = ready();
    c.double_click("b", true).unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::Open {
            id: "b".into(),
            new_window: true
        }
    );
}

#[test]
fn rename_return_escape_empty_and_unchanged_match_finder_behavior() {
    let mut c = ready();
    let count = c.port.sent.len();
    c.menu("a", MenuAction::Rename).unwrap();
    c.rename.as_mut().unwrap().text = "Changed".into();
    c.key(Key::Escape, 1).unwrap();
    assert!(c.rename.is_none());
    assert_eq!(c.port.sent.len(), count);
    for value in ["", "   ", "Launch Film"] {
        c.menu("a", MenuAction::Rename).unwrap();
        c.rename.as_mut().unwrap().text = value.into();
        c.key(Key::Return, 1).unwrap();
        assert_eq!(c.port.sent.len(), count);
    }
    c.menu("a", MenuAction::Rename).unwrap();
    c.rename.as_mut().unwrap().text = "  New Film  ".into();
    c.key(Key::Return, 1).unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::Rename {
            id: "a".into(),
            name: "New Film".into()
        }
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
fn trash_requires_confirmation_and_undo_only_appears_after_success() {
    let mut c = ready();
    let count = c.port.sent.len();
    c.menu("a", MenuAction::Trash).unwrap();
    let (serial, prompt) = alert(&mut c);
    assert_eq!(prompt.message, "Move “Launch Film” to the Trash?");
    assert_eq!(prompt.detail, "You can put it back from Recently Deleted.");
    assert_eq!(prompt.buttons, vec!["Move to Trash", "Cancel"]);
    assert_eq!(prompt.default, 0);
    assert!(!prompt.critical);
    assert_eq!(c.port.sent.len(), count);
    assert_eq!(c.undo_label(), None);
    c.answer(serial, Some(0)).unwrap();
    let serial = c.port.sent.last().unwrap().serial;
    complete(&mut c, serial, Ok(Outcome::Changed));
    assert_eq!(c.undo_label(), Some("Undo Move to Trash"));
    c.undo_move_to_trash().unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::PutBack("a".into())
    );
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
    assert_eq!(prompt.message, "Delete “Unused” immediately?");
    assert_eq!(
        prompt.detail,
        "The project is archived in Powermove’s storage and can’t be opened from the Library again."
    );
    assert_eq!(prompt.buttons, vec!["Cancel", "Delete"]);
    assert_eq!(prompt.default, 0);
    assert_eq!(prompt.destructive, Some(1));
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
    assert_eq!(c.answer(serial + 1, Some(0)), Err(Failure::Stale));
    assert_eq!(c.answer(serial, Some(9)), Err(Failure::Stale));
    c.answer(serial, None).unwrap();
    assert!(
        c.port
            .sent
            .iter()
            .all(|r| !matches!(r.command, Command::Trash(_)))
    );
    assert_eq!(c.answer(serial, Some(0)), Err(Failure::Stale));
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
fn trash_subtitle_and_first_save_choices_match_spec() {
    let mut c = ready();
    assert!(!c.before_save("a").unwrap());
    assert!(c.before_save("c").unwrap());
    let all = effects(&mut c);
    assert!(
        all.iter()
            .any(|e| matches!(e,HostEffect::Subtitle { id,text: Some("In Trash") } if id == "c"))
    );
    let (serial, prompt) = all
        .into_iter()
        .find_map(|e| {
            if let HostEffect::Alert { serial, alert, .. } = e {
                Some((serial, alert))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(prompt.message, "“Unused” is in the Trash.");
    assert!(prompt.detail.is_empty());
    assert_eq!(
        prompt.buttons,
        vec!["Put Back and Save", "Save As…", "Cancel"]
    );
    c.answer(serial, Some(1)).unwrap();
    assert!(
        effects(&mut c)
            .iter()
            .any(|e| matches!(e,HostEffect::SaveAs(id) if id == "c"))
    );
    assert!(c.before_save("c").unwrap());
    let (serial, _) = alert(&mut c);
    c.answer(serial, Some(0)).unwrap();
    assert_eq!(
        c.port.sent.last().unwrap().command,
        Command::PutBackAndSave("c".into())
    );
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
    assert_eq!(f.linked, None);
    let f = NewProjectForm::new("owner".into(), None, &format(), &[]);
    assert_eq!(f.name, "Untitled");
    assert_eq!(f.values().unwrap().format, format());
}

#[test]
fn preset_copy_dimensions_and_manual_edits_switch_to_custom() {
    assert_eq!(
        PRESETS.iter().map(|p| p.0).collect::<Vec<_>>(),
        vec![
            "4K UHD · 3840×2160",
            "HD 1080p · 1920×1080",
            "HD 720p · 1280×720",
            "Vertical HD · 1080×1920",
            "Portrait 4:5 · 1080×1350",
            "Square · 1080×1080"
        ]
    );
    let mut f = NewProjectForm::new("owner".into(), None, &format(), &[]);
    for (index, (_, w, h)) in PRESETS.iter().enumerate() {
        f.choose_preset(Some(index)).unwrap();
        assert_eq!(f.width, w.to_string());
        assert_eq!(f.height, h.to_string());
    }
    f.edit_size("1920".into(), "1080".into()).unwrap();
    assert_eq!(f.preset, None);
}

#[test]
fn sheet_fields_and_voiceover_labels_preserve_the_shipped_contract() {
    assert_eq!(
        FORM_ROWS,
        &[
            "Name",
            "Resolution",
            "Size",
            "Frame Rate",
            "Duration",
            "Background"
        ]
    );
    assert_eq!(
        FIELD_LABELS,
        &[
            "Name",
            "Resolution",
            "Width",
            "Height",
            "Frame Rate",
            "Duration",
            "Background"
        ]
    );
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
    complete(&mut c, serial, Err(Failure::Service(ErrorCode::DiskFull)));
    assert_eq!(c.form.as_ref().unwrap().name, "Launch Film");
    assert!(!c.form.as_ref().unwrap().submitting);
    let (serial, prompt) = alert(&mut c);
    assert_eq!(prompt.message, "There isn’t enough disk space.");
    c.answer(serial, Some(0)).unwrap();
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
fn stale_sheet_owner_closes_with_exact_banner_and_escape_never_creates() {
    let mut c = ready();
    c.begin_new("owner:1".into(), None, &format()).unwrap();
    c.owner_changed("owner:2");
    assert!(c.form.is_some());
    c.owner_changed("owner:1");
    assert!(c.form.is_none());
    assert!(effects(&mut c).iter().any(|e| matches!(
        e,
        HostEffect::Banner("The project was changed, so the dialog closed.")
    )));
    c.begin_new("owner:2".into(), None, &format()).unwrap();
    let count = c.port.sent.len();
    c.key(Key::Escape, 1).unwrap();
    assert!(c.form.is_none());
    assert_eq!(c.port.sent.len(), count);
}

#[test]
fn service_errors_use_spec_sentences_and_never_show_diagnostics() {
    let cases = [
        (
            ErrorCode::Offline,
            "Powermove can’t connect.",
            "Check your internet connection and try again.",
        ),
        (
            ErrorCode::NotFound,
            "“Launch Film” can’t be found.",
            "It may have been moved or deleted.",
        ),
        (
            ErrorCode::TooLarge,
            "“Launch Film” is too large to open.",
            "",
        ),
        (
            ErrorCode::InvalidProject,
            "“Launch Film” can’t be opened because it isn’t a Powermove project.",
            "",
        ),
        (
            ErrorCode::PermissionDenied,
            "You don’t have permission to open “Launch Film”.",
            "Check its sharing and permissions in Finder.",
        ),
        (
            ErrorCode::DiskFull,
            "There isn’t enough disk space.",
            "Free some space and try again.",
        ),
        (
            ErrorCode::InvalidInput,
            "“Launch Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            ErrorCode::InvalidData,
            "“Launch Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            ErrorCode::Unavailable,
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            ErrorCode::Unknown,
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    ];
    for (code, message, detail) in cases {
        let alert = Alert::error(code, "Launch Film");
        assert_eq!(alert.message, message);
        assert_eq!(alert.detail, detail);
        assert_eq!(alert.buttons, vec!["OK"]);
        assert!(!alert.critical);
    }
}

#[test]
fn unresolved_tokens_stay_symbolic_and_resolved_metrics_follow_theme() {
    let mut tokens = metrics();
    let m = WindowMetrics::resolve(&tokens);
    assert_eq!(m.min_width, tokens.metrics.min_width);
    assert_eq!(m.min_height, tokens.metrics.min_height / 1.25);
    assert_eq!(m.sidebar, tokens.metrics.settings_sidebar);
    assert_eq!(m.thumbnail_min, "metrics.thumbnail_min");
    assert_eq!(m.sheet_inset, "metrics.sheet_inset");
    assert_eq!(m.sheet_value_column, "metrics.sheet_value_column");
    assert_eq!(m.media_badge, "colors.media_badge");
    tokens.apply_patch([("metrics.row", 32.0)]).unwrap();
    assert_eq!(WindowMetrics::resolve(&tokens).row, 32.0);
}

#[test]
fn disconnected_service_and_unsupported_operations_are_not_successes() {
    let mut c = LibraryController::new(Port {
        refuse: true,
        ..Default::default()
    });
    assert!(c.show().is_err());
    assert!(matches!(
        c.presentation(Duration::ZERO, 0..0).content,
        Content::Unavailable { .. }
    ));
    let mut c = ready();
    c.menu("c", MenuAction::DeleteImmediately).unwrap();
    let (serial, _) = alert(&mut c);
    c.answer(serial, Some(1)).unwrap();
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
    let Outcome::Opened {
        reply: pm_kernel::session::ShellReply::Opened { session, .. },
        ..
    } = service
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
        .execute(Command::Open {
            id: session.project.clone(),
            new_window: true,
        })
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
fn form_failure_and_first_save_alerts_keep_their_owning_windows() {
    let mut c = ready();
    c.begin_new("editor:7".into(), None, &format()).unwrap();
    c.create().unwrap();
    let serial = c.port.sent.last().unwrap().serial;
    complete(&mut c, serial, Err(Failure::Service(ErrorCode::DiskFull)));
    let list = effects(&mut c);
    assert!(list.iter().any(|e| matches!(e,HostEffect::Alert { owner: AlertOwner::NewProject(owner),.. } if owner == "editor:7")));
    let mut c = ready();
    c.before_save("c").unwrap();
    assert!(
        effects(&mut c).iter().any(
            |e| matches!(e,HostEffect::Alert { owner: AlertOwner::Editor(id),.. } if id == "c")
        )
    );
}

#[test]
fn modal_alert_focus_scope_blocks_underlying_keyboard_actions() {
    let mut c = ready();
    c.select("a").unwrap();
    c.menu("a", MenuAction::Trash).unwrap();
    let count = c.port.sent.len();
    c.key(Key::Return, 1).unwrap();
    c.key(Key::NewProject, 1).unwrap();
    assert_eq!(c.port.sent.len(), count);
    assert!(c.form.is_none());
}

#[test]
fn sheet_contract_contains_default_button_focus_selection_and_units() {
    assert_eq!(SHEET_TITLE, "New Project");
    assert_eq!(SHEET_BUTTONS, &["Cancel", "Create"]);
    assert_eq!(CUSTOM_PRESET, "Custom");
    assert_eq!(UNITS, &["px", "fps", "s"]);
    assert_eq!(INITIAL_FOCUS, "Name");
    let mut c = ready();
    c.begin_new("owner".into(), None, &format()).unwrap();
    let form = c.presentation(Duration::ZERO, 0..1).form.unwrap();
    assert_eq!(form.owner, "owner");
    assert!(form.can_create());
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
    let Outcome::Opened {
        reply: pm_kernel::session::ShellReply::Opened { session, .. },
        ..
    } = service.execute(Command::Create(input.clone())).unwrap()
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
fn external_put_back_removes_stale_library_undo_action() {
    let mut c = ready();
    c.menu("a", MenuAction::Trash).unwrap();
    let (serial, _) = alert(&mut c);
    c.answer(serial, Some(0)).unwrap();
    let serial = c.port.sent.last().unwrap().serial;
    complete(&mut c, serial, Ok(Outcome::Changed));
    assert_eq!(c.undo_label(), Some("Undo Move to Trash"));
    load(&mut c, sample());
    assert_eq!(c.undo_label(), None);
}

#[test]
fn name_sort_calls_the_host_collator_instead_of_assuming_ascii_order() {
    let snapshot = sample();
    let query = Query {
        sort: Sort::Name,
        ..Default::default()
    };
    let rows = query.apply_with_names(&snapshot, |a, b| b.cmp(a));
    assert_eq!(rows[0].name, "Trailer");
}

#[test]
fn real_kernel_open_hands_a_closed_project_to_the_editor_with_new_window_intent() {
    let (_dir, actor, mut service) = service();
    let session = create_real(&mut service);
    service
        .kernel
        .shell(pm_kernel::session::ShellCall::Close(session.clone()))
        .wait()
        .unwrap()
        .unwrap();
    let result = service
        .execute(Command::Open {
            id: session.project.clone(),
            new_window: true,
        })
        .unwrap();
    let Outcome::Opened {
        reply: pm_kernel::session::ShellReply::Opened {
            session: opened, ..
        },
        new_window,
    } = result
    else {
        panic!("opened");
    };
    assert!(new_window);
    assert_eq!(opened.project, session.project);
    assert_ne!(opened.generation, session.generation);
    actor.shutdown();
}

#[test]
fn stale_owner_closes_even_when_creation_is_in_flight() {
    let mut c = ready();
    c.begin_new("editor:1".into(), None, &format()).unwrap();
    c.create().unwrap();
    c.owner_changed("editor:1");
    assert!(c.form.is_none());
    assert_eq!(c.create(), Err(Failure::Stale));
}
