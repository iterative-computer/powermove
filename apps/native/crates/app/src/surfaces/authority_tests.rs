use super::*;
use values::*;

fn target(window: &str) -> Target {
    Target {
        window: window.into(),
        object: "extension-id".into(),
        generation: 4,
    }
}
fn private(text: &str) -> PrivateValue {
    PrivateValue::new(text.into())
}
fn full() -> Prompt {
    Prompt::FullAccess {
        thread: "thread-id".into(),
        effect: 12,
    }
}
fn run(summary: &str) -> Prompt {
    Prompt::RunAccess {
        run: "run-id".into(),
        project: "Film".into(),
        summary: summary.into(),
    }
}
fn uninstall() -> Prompt {
    Prompt::Uninstall {
        extension: "extension-id".into(),
        name: "Extension".into(),
        revision: 9,
    }
}
fn workspace() -> Prompt {
    Prompt::DeleteWorkspace {
        workspace: "workspace-id".into(),
        name: "Editing".into(),
        revision: 6,
    }
}
fn alert(prompt: Prompt) -> Alert {
    Alert {
        id: 1,
        target: target("w"),
        prompt,
    }
}

#[derive(Default)]
struct Services {
    commands: Vec<Command>,
    fail: bool,
}
impl AuthorityServices for Services {
    fn enqueue(&mut self, command: Command) -> Result<(), ServiceCode> {
        if self.fail {
            Err(ServiceCode::Unavailable)
        } else {
            self.commands.push(command);
            Ok(())
        }
    }
}
#[derive(Default)]
struct Host {
    shown: Vec<(RequestId, String)>,
    dismissed: Vec<RequestId>,
    banners: Vec<String>,
    copied: Vec<String>,
}
impl AuthorityHost for Host {
    fn present(&mut self, view: AlertView<'_>) {
        self.shown.push((view.id, view.message));
    }
    fn dismiss(&mut self, _: &str, id: RequestId) {
        self.dismissed.push(id);
    }
    fn copy_concealed(&mut self, value: &PrivateValue) {
        self.copied.push(value.expose().into());
    }
    fn changed_banner(&mut self, _: &Target, text: &str) {
        self.banners.push(text.into());
    }
}
fn enqueue(a: &mut Authority, s: &mut Services, h: &mut Host, w: &str, p: Prompt) -> RequestId {
    a.request(target(w), p, s, h).unwrap()
}
#[test]
fn standing_full_access_exact_copy_and_keyboard() {
    let a = alert(full());
    let v = a.view();
    assert_eq!(v.message, "Give Powermove agents full access to your Mac?");
    assert_eq!(
        v.informative.expose(),
        "Agents will run commands and change files anywhere on this Mac without asking, until you choose another access level or quit Powermove."
    );
    assert_eq!(v.style, AlertStyle::Critical);
    assert_eq!(
        v.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Cancel", "Allow Full Access"]
    );
    assert!(v.buttons[0].default && v.buttons[0].escape);
    assert!(!v.buttons[1].default);
    assert!(v.close_access_popover_first);
}
#[test]
fn per_project_exact_copy_plain_summary_and_visual_line_limit() {
    let a = alert(run("<b>Command</b>\nsecond\nthird\nfourth"));
    let v = a.view();
    assert_eq!(v.message, "Allow computer access for “Film”?");
    assert_eq!(
        v.informative.expose(),
        "<b>Command</b>\nsecond\nthird\nfourth"
    );
    assert_eq!(v.max_lines, Some(3));
    assert_eq!(v.style, AlertStyle::Warning);
    assert_eq!(
        v.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Cancel", "Allow This Run"]
    );
    assert!(v.buttons[0].default);
    assert!(!v.close_access_popover_first);
}
#[test]
fn full_summary_is_delivered_to_the_activity_row_without_debug_leak() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    enqueue(
        &mut a,
        &mut s,
        &mut h,
        "w",
        run("synthetic private summary"),
    );
    assert!(!format!("{:?}", s.commands).contains("synthetic private summary"));
    match &s.commands[0] {
        Command::AwaitingAnswer {
            run,
            activity_summary,
            ..
        } => {
            assert_eq!(run.as_deref(), Some("run-id"));
            assert_eq!(
                activity_summary.as_ref().unwrap().expose(),
                "synthetic private summary"
            );
        }
        _ => panic!("awaiting state"),
    }
    assert!(a.inspect("w")[0].awaiting_answer);
}
#[test]
fn cancellation_denies_the_exact_run() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let id = enqueue(&mut a, &mut s, &mut h, "w", run("summary"));
    a.answer(&target("w"), id, Action::Cancel, &mut s, &mut h)
        .unwrap();
    assert!(
        matches!(s.commands.last(), Some(Command::RunDecision { run, granted: false, .. }) if run == "run-id")
    );
}
#[test]
fn standing_consent_preserves_panel_effect_id() {
    for (action, expected) in [(Action::Cancel, false), (Action::AllowFullAccess, true)] {
        let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
        let id = enqueue(&mut a, &mut s, &mut h, "w", full());
        a.answer(&target("w"), id, action, &mut s, &mut h).unwrap();
        assert!(
            matches!(s.commands.last(), Some(Command::ComputerConsent { thread, effect:12, granted, .. }) if thread == "thread-id" && *granted == expected)
        );
    }
}
#[test]
fn per_run_allow_does_not_create_standing_consent() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let id = enqueue(&mut a, &mut s, &mut h, "w", run("summary"));
    a.answer(&target("w"), id, Action::AllowThisRun, &mut s, &mut h)
        .unwrap();
    assert!(matches!(
        s.commands.last(),
        Some(Command::RunDecision { granted: true, .. })
    ));
    assert!(
        !s.commands
            .iter()
            .any(|c| matches!(c, Command::ComputerConsent { .. }))
    );
}
#[test]
fn one_alert_per_window_fifo_and_other_windows_are_independent() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let first = enqueue(&mut a, &mut s, &mut h, "a", full());
    let second = enqueue(&mut a, &mut s, &mut h, "a", workspace());
    enqueue(&mut a, &mut s, &mut h, "b", full());
    assert_eq!(h.shown.len(), 2);
    assert_eq!(
        a.answer(&target("a"), second, Action::Cancel, &mut s, &mut h),
        Err(Error::Stale)
    );
    a.answer(&target("a"), first, Action::Cancel, &mut s, &mut h)
        .unwrap();
    assert_eq!(h.shown.last().unwrap().0, second);
    assert_eq!(a.inspect("b").len(), 1);
}
#[test]
fn duplicate_native_completion_cannot_answer_the_next_alert() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let first = enqueue(&mut a, &mut s, &mut h, "w", full());
    let second = enqueue(&mut a, &mut s, &mut h, "w", full());
    a.answer(&target("w"), first, Action::Cancel, &mut s, &mut h)
        .unwrap();
    assert_eq!(
        a.answer(&target("w"), first, Action::AllowFullAccess, &mut s, &mut h),
        Err(Error::Stale)
    );
    assert_eq!(a.inspect("w")[0].id, second);
}
#[test]
fn changed_generation_or_wrong_action_cannot_grant() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let id = enqueue(&mut a, &mut s, &mut h, "w", full());
    let mut wrong = target("w");
    wrong.generation += 1;
    assert_eq!(
        a.answer(&wrong, id, Action::AllowFullAccess, &mut s, &mut h),
        Err(Error::Stale)
    );
    assert_eq!(
        a.answer(&target("w"), id, Action::AllowThisRun, &mut s, &mut h),
        Err(Error::InvalidAction)
    );
    assert_eq!(s.commands.len(), 1);
}
#[test]
fn worker_queue_failure_retains_alert_for_retry() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let id = enqueue(&mut a, &mut s, &mut h, "w", full());
    s.fail = true;
    assert_eq!(
        a.answer(&target("w"), id, Action::AllowFullAccess, &mut s, &mut h),
        Err(Error::Service(ServiceCode::Unavailable))
    );
    assert_eq!(a.inspect("w")[0].id, id);
    assert!(h.dismissed.is_empty());
    s.fail = false;
    a.answer(&target("w"), id, Action::Cancel, &mut s, &mut h)
        .unwrap();
}
#[test]
fn awaiting_publication_failure_never_opens_an_alert() {
    let (mut a, mut s, mut h) = (
        Authority::default(),
        Services {
            fail: true,
            ..Services::default()
        },
        Host::default(),
    );
    assert!(a.request(target("w"), full(), &mut s, &mut h).is_err());
    assert!(h.shown.is_empty());
    assert!(a.inspect("w").is_empty());
}
#[test]
fn disappearing_target_denies_active_and_queued_requests_and_posts_exact_banner() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let id = enqueue(&mut a, &mut s, &mut h, "w", full());
    enqueue(&mut a, &mut s, &mut h, "w", run("summary"));
    a.invalidate(&target("w"), "Project", &mut s, &mut h)
        .unwrap();
    assert!(a.inspect("w").is_empty());
    assert_eq!(h.dismissed, [id]);
    assert_eq!(h.banners, ["Project was changed, so the dialog closed."]);
    assert!(matches!(
        s.commands.last(),
        Some(Command::RunDecision { granted: false, .. })
    ));
}
#[test]
fn standing_access_expires_on_quit_without_losing_previous_non_full_level() {
    for level in [AccessLevel::Editor, AccessLevel::Project] {
        let mut s = StandingAccess::launch(level);
        s.allow_full();
        assert_eq!(s.current(), AccessLevel::Full);
        assert_eq!(s.saved_level(), level);
        assert_eq!(
            s.full_indicator(),
            Some((
                "Full access",
                "exclamationmark.triangle.fill",
                "colors.warning"
            ))
        );
        assert_eq!(StandingAccess::launch(s.saved_level()).current(), level);
        s.select_non_full(AccessLevel::Editor).unwrap();
        assert_eq!(s.full_indicator(), None);
        assert_eq!(
            s.select_non_full(AccessLevel::Full),
            Err(Error::InvalidAction)
        );
    }
    assert_ne!(
        StandingAccess::launch(AccessLevel::Full).current(),
        AccessLevel::Full
    );
}
#[test]
fn reveal_exact_copy_is_explicit_and_not_inspectable() {
    let a = alert(Prompt::Reveal {
        key: "key".into(),
        label: "API Key".into(),
        value: private("synthetic private value"),
    });
    let v = a.view();
    assert_eq!(v.message, "API Key");
    assert_eq!(v.informative.expose(), "synthetic private value");
    assert_eq!(
        v.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Done", "Copy"]
    );
    assert!(v.buttons[0].default);
    assert!(!format!("{a:?} {v:?} {:?}", a.inspection()).contains("synthetic private value"));
}
#[test]
fn reveal_done_does_not_copy_and_copy_uses_concealed_host_path() {
    for action in [Action::Done, Action::Copy] {
        let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
        let id = enqueue(
            &mut a,
            &mut s,
            &mut h,
            "w",
            Prompt::Reveal {
                key: "key".into(),
                label: "API Key".into(),
                value: private("synthetic"),
            },
        );
        a.answer(&target("w"), id, action, &mut s, &mut h).unwrap();
        assert_eq!(h.copied.len(), usize::from(action == Action::Copy));
        assert!(s.commands.is_empty());
    }
}
#[test]
fn uninstall_exact_copy_safe_default_and_destructive_delete() {
    let a = alert(uninstall());
    let v = a.view();
    assert_eq!(
        v.message,
        "Also delete the values you entered for Extension?"
    );
    assert_eq!(
        v.informative.expose(),
        "If you keep them, they’re used again when you add this extension back."
    );
    assert_eq!(
        v.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Keep", "Delete"]
    );
    assert!(v.buttons[0].default);
    assert!(v.buttons[1].destructive && !v.buttons[1].default);
}
#[test]
fn uninstall_keep_and_delete_both_archive_release_and_only_delete_erases_values() {
    for action in [Action::Keep, Action::Delete] {
        let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
        let id = enqueue(&mut a, &mut s, &mut h, "w", uninstall());
        a.answer(&target("w"), id, action, &mut s, &mut h).unwrap();
        assert!(
            matches!(s.commands.last(), Some(Command::ArchiveExtension { extension, revision:9, delete_values, .. }) if extension == "extension-id" && *delete_values == (action == Action::Delete))
        );
    }
}
#[test]
fn no_values_uninstall_has_no_confirmation_and_keeps_values_policy() {
    let (mut a, mut s) = (Authority::default(), Services::default());
    a.uninstall_without_values(target("w"), "id".into(), 1, &mut s)
        .unwrap();
    assert!(a.inspect("w").is_empty());
    assert!(matches!(
        s.commands[0],
        Command::ArchiveExtension {
            delete_values: false,
            ..
        }
    ));
}
#[test]
fn workspace_exact_copy_button_order_cancel_default_and_archive_command() {
    let a = alert(workspace());
    let v = a.view();
    assert_eq!(v.message, "Delete the workspace “Editing”?");
    assert_eq!(
        v.informative.expose(),
        "Your panels stay as they are. You can undo this."
    );
    assert_eq!(
        v.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        ["Delete", "Cancel"]
    );
    assert!(v.buttons[0].destructive && !v.buttons[0].default);
    assert!(v.buttons[1].default && v.buttons[1].escape);
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let id = enqueue(&mut a, &mut s, &mut h, "w", workspace());
    a.answer(&target("w"), id, Action::Delete, &mut s, &mut h)
        .unwrap();
    assert!(matches!(
        s.commands[0],
        Command::ArchiveWorkspace { revision: 6, .. }
    ));
}
#[test]
fn workspace_cancel_leaves_layout_and_service_untouched() {
    let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
    let id = enqueue(&mut a, &mut s, &mut h, "w", workspace());
    a.answer(&target("w"), id, Action::Cancel, &mut s, &mut h)
        .unwrap();
    assert!(s.commands.is_empty());
}
#[test]
fn local_project_trash_and_permanent_archive_exact_copy() {
    for permanent in [false, true] {
        let p = if permanent {
            Prompt::DeleteLocal {
                project: "id".into(),
                name: "Panel".into(),
            }
        } else {
            Prompt::TrashLocal {
                project: "id".into(),
                name: "Panel".into(),
            }
        };
        let a = alert(p);
        let v = a.view();
        if permanent {
            assert_eq!(v.message, "Delete “Panel” immediately?");
            assert_eq!(
                v.informative.expose(),
                "The project is archived in Powermove’s storage and can’t be opened from the Library again."
            );
            assert_eq!(v.style, AlertStyle::Critical);
            assert!(v.buttons[1].default);
        } else {
            assert_eq!(v.message, "Move “Panel” to the Trash?");
            assert_eq!(
                v.informative.expose(),
                "You can put it back from Recently Deleted."
            );
            assert_eq!(v.buttons[0].label, "Move to Trash");
            assert!(v.buttons[0].default);
        }
    }
}

#[derive(Default)]
struct ValueService {
    saves: Vec<Save>,
    removes: Vec<(Target, RequestId, String, String)>,
    fail: bool,
}
impl ValuesService for ValueService {
    fn enqueue_save(&mut self, save: Save) -> Result<(), (ServiceCode, Save)> {
        if self.fail {
            Err((ServiceCode::DiskFull, save))
        } else {
            self.saves.push(save);
            Ok(())
        }
    }
    fn enqueue_remove(
        &mut self,
        target: Target,
        request: RequestId,
        env: String,
        key: String,
    ) -> Result<(), ServiceCode> {
        if self.fail {
            return Err(ServiceCode::Unavailable);
        }
        self.removes.push((target, request, env, key));
        Ok(())
    }
}
fn declaration(key: &str, kind: Kind, required: bool) -> Declaration {
    Declaration {
        key: key.into(),
        label: if key == "token" {
            "API Key"
        } else {
            "Endpoint"
        }
        .into(),
        kind,
        required,
        help: Some("Declared help.".into()),
    }
}
fn setup(secret: SavedValue, required: bool) -> Setup {
    Setup::open(
        target("w"),
        "Extension".into(),
        "provenance-env-key".into(),
        vec![
            (
                declaration("endpoint", Kind::Plain, false),
                SavedValue::Plain("https://example.test".into()),
            ),
            (declaration("token", Kind::Secret, required), secret),
        ],
        Origin::ExtensionSettings,
    )
    .unwrap()
}
#[test]
fn setup_exact_title_required_line_footer_note_and_keyboard_contract() {
    let s = setup(SavedValue::Missing, true);
    let v = s.view();
    assert_eq!(v.title, "Set Up Extension");
    assert_eq!(
        v.explanation.as_deref(),
        Some("Extension needs these values to work.")
    );
    assert_eq!(
        v.note,
        "Stays on this Mac. Never included when you publish or share this extension."
    );
    assert_eq!((v.cancel_label, v.save_label), ("Cancel", "Save"));
    assert!(v.focus_first_field_and_select && v.return_saves && v.escape_cancels);
    assert!(!v.can_save);
    assert!(v.can_cancel);
}
#[test]
fn optional_rows_help_declared_labels_and_native_widget_kinds() {
    let v = setup(SavedValue::Missing, false).view();
    assert_eq!(v.explanation, None);
    assert!(v.can_save);
    assert_eq!(v.rows[0].widget, Widget::NSTextField);
    assert_eq!(v.rows[0].label, "Endpoint");
    assert_eq!(v.rows[0].optional, Some("Optional"));
    assert_eq!(v.rows[0].help.as_deref(), Some("Declared help."));
    assert_eq!(v.rows[1].widget, Widget::NSSecureTextField);
    assert_eq!(v.rows[1].accessibility_label, "API Key");
}
#[test]
fn already_set_secret_has_exact_mask_accessibility_and_actions_without_value() {
    let s = setup(SavedValue::SecretSet, true);
    let v = s.view();
    let r = &v.rows[1];
    assert!(v.can_save);
    assert_eq!(v.explanation, None);
    assert_eq!(r.widget, Widget::AlreadySet);
    assert_eq!(r.display_marker, Some("••••"));
    assert_eq!(r.accessibility_label, "API Key is set");
    assert_eq!(r.actions, ["Reveal", "Change", "Remove"]);
    assert_eq!(r.eye_tooltip, None);
    assert!(s.field_value("token").is_err());
}
#[test]
fn unreadable_keychain_item_requires_reentry_with_exact_copy_and_empty_field() {
    let s = setup(SavedValue::Unreadable, true);
    let v = s.view();
    assert_eq!(
        v.rows[1].warning,
        Some("Enter this again; the saved value can’t be read.")
    );
    assert_eq!(s.field_value("token").unwrap(), "");
    assert!(!v.can_save);
}
#[test]
fn secret_eye_toggles_only_current_draft_and_exact_labels() {
    let mut s = setup(SavedValue::Missing, true);
    assert_eq!(
        s.view().rows[1].eye_tooltip.as_deref(),
        Some("Show API Key")
    );
    s.edit("token", private("synthetic private value")).unwrap();
    s.toggle_visibility("token").unwrap();
    let v = s.view();
    assert_eq!(v.rows[1].widget, Widget::NSTextField);
    assert_eq!(v.rows[1].eye_tooltip.as_deref(), Some("Hide API Key"));
    assert_eq!(v.rows[1].eye_symbol, Some("eye"));
    assert!(!format!("{s:?} {v:?}").contains("synthetic private value"));
    s.toggle_visibility("token").unwrap();
    assert_eq!(s.view().rows[1].widget, Widget::NSSecureTextField);
    assert!(s.toggle_visibility("endpoint").is_err());
}
#[test]
fn change_secret_never_prefills_a_placeholder_or_stored_value() {
    let mut s = setup(SavedValue::SecretSet, true);
    assert!(s.edit("token", private("synthetic")).is_err());
    assert!(s.toggle_visibility("token").is_err());
    s.change("token").unwrap();
    assert_eq!(s.field_value("token").unwrap(), "");
    assert_eq!(s.view().rows[1].display_marker, None);
    assert!(!s.view().can_save);
}
#[test]
fn required_presence_validates_live_and_keeps_authored_bytes() {
    let mut s = setup(SavedValue::Missing, true);
    let mut svc = ValueService::default();
    s.edit("token", private("")).unwrap();
    assert!(!s.view().can_save);
    assert!(s.save(1, &mut svc).is_err());
    s.edit("token", private("  synthetic exact bytes\n"))
        .unwrap();
    assert!(s.view().can_save);
    s.save(1, &mut svc).unwrap();
    match &svc.saves[0].writes[1] {
        Write::Set {
            value,
            kind: Kind::Secret,
            ..
        } => assert_eq!(value.expose(), "  synthetic exact bytes\n"),
        _ => panic!("secret write"),
    }
    assert_eq!(svc.saves[0].env_key, "provenance-env-key");
    assert!(!format!("{:?}", svc.saves).contains("synthetic exact bytes"));
}
#[test]
fn saving_disables_edits_and_duplicate_submit_until_durable_completion() {
    let mut s = setup(SavedValue::SecretSet, true);
    let mut svc = ValueService::default();
    s.save(1, &mut svc).unwrap();
    let v = s.view();
    assert_eq!(v.save_label, "Saving…");
    assert!(!v.can_save && v.can_cancel);
    assert_eq!(s.change("token"), Err(Error::Busy));
    assert_eq!(s.save(2, &mut svc), Err(Error::Busy));
    s.complete(&target("w"), 1, Ok(())).unwrap();
    assert!(s.closed());
    assert!(s.field_value("endpoint").is_err());
    assert!(s.cancel().is_err());
}
#[test]
fn failed_save_keeps_typed_secret_and_renders_only_service_code_copy() {
    let mut s = setup(SavedValue::Missing, true);
    let mut svc = ValueService::default();
    s.edit("token", private("synthetic draft")).unwrap();
    s.save(1, &mut svc).unwrap();
    s.complete(&target("w"), 1, Err(ServiceCode::DiskFull))
        .unwrap();
    assert_eq!(
        s.view().error.as_deref(),
        Some("Couldn’t save these values. Free some space and try again.")
    );
    assert_eq!(s.field_value("token").unwrap(), "synthetic draft");
    assert!(s.view().can_save);
    assert!(s.save(1, &mut svc).is_err());
    s.save(2, &mut svc).unwrap();
}
#[test]
fn queue_full_preserves_draft_and_does_not_enter_saving() {
    let mut s = setup(SavedValue::Missing, true);
    let mut svc = ValueService {
        fail: true,
        ..ValueService::default()
    };
    s.edit("token", private("synthetic draft")).unwrap();
    assert_eq!(
        s.save(1, &mut svc),
        Err(Error::Service(ServiceCode::DiskFull))
    );
    assert_eq!(s.view().save_label, "Save");
    assert_eq!(
        s.view().error.as_deref(),
        Some("Couldn’t save these values. Free some space and try again.")
    );
    assert_eq!(s.field_value("token").unwrap(), "synthetic draft");
    svc.fail = false;
    s.save(1, &mut svc).unwrap();
}
#[test]
fn stale_save_completion_cannot_close_a_newer_target_or_request() {
    let mut s = setup(SavedValue::SecretSet, true);
    let mut svc = ValueService::default();
    s.save(1, &mut svc).unwrap();
    let mut wrong = target("w");
    wrong.generation += 1;
    assert_eq!(s.complete(&wrong, 1, Ok(())), Err(Error::Stale));
    assert_eq!(s.complete(&target("w"), 2, Ok(())), Err(Error::Stale));
    assert!(!s.closed());
    s.complete(&target("w"), 1, Ok(())).unwrap();
    assert_eq!(s.complete(&target("w"), 1, Ok(())), Err(Error::Stale));
}
#[test]
fn remove_is_immediate_without_confirmation_and_cancel_never_resurrects() {
    let mut s = setup(SavedValue::SecretSet, true);
    let mut svc = ValueService::default();
    s.remove("token", 1, &mut svc).unwrap();
    assert_eq!(svc.removes.len(), 1);
    s.complete_remove(&target("w"), 1, Ok(())).unwrap();
    assert_eq!(s.field_value("token").unwrap(), "");
    assert!(!s.view().can_save);
    s.cancel().unwrap();
    assert!(svc.saves.is_empty());
    assert_eq!(svc.removes.len(), 1);
}
#[test]
fn failed_or_stale_remove_preserves_saved_status_and_allows_retry() {
    let mut s = setup(SavedValue::SecretSet, true);
    let mut svc = ValueService::default();
    s.remove("token", 1, &mut svc).unwrap();
    assert_eq!(
        s.complete_remove(&target("w"), 2, Ok(())),
        Err(Error::Stale)
    );
    assert_eq!(
        s.complete_remove(&target("w"), 1, Err(ServiceCode::PermissionDenied)),
        Err(Error::Service(ServiceCode::PermissionDenied))
    );
    assert_eq!(s.view().rows[1].display_marker, Some("••••"));
    s.remove("token", 2, &mut svc).unwrap();
}
#[test]
fn explicit_reveal_uses_env_key_and_declared_label_without_fetching_a_secret() {
    let s = setup(SavedValue::SecretSet, true);
    let (_, env, key, label) = s.reveal_key("token").unwrap();
    assert_eq!(
        (env, key, label),
        ("provenance-env-key", "token", "API Key")
    );
    assert!(s.reveal_key("endpoint").is_err());
}
#[test]
fn missing_required_panel_state_exact_copy_and_kit_component() {
    let v = missing_values("Extension");
    assert_eq!(v.title, "Set Up Extension");
    assert_eq!(v.line, "Extension needs a few values before it can run.");
    assert_eq!(v.button, "Set Up…");
    assert_eq!(v.component, "EmptyState");
}
#[test]
fn setup_entry_points_share_model_and_agent_secret_question_never_opens_sheet() {
    for origin in [
        Origin::ExtensionSettings,
        Origin::MissingPanel,
        Origin::RequestValues,
    ] {
        assert!(Setup::open(target("w"), "E".into(), "env".into(), vec![], origin).is_ok());
    }
    assert!(matches!(
        Setup::open(
            target("w"),
            "E".into(),
            "env".into(),
            vec![],
            Origin::AgentSecretQuestion
        ),
        Err(Error::InvalidAction)
    ));
}
#[test]
fn unknown_duplicate_or_wrongly_typed_declarations_are_rejected() {
    assert!(
        Setup::open(
            target("w"),
            "E".into(),
            "env".into(),
            vec![(
                declaration("token", Kind::Secret, true),
                SavedValue::Plain("bad".into())
            )],
            Origin::RequestValues
        )
        .is_err()
    );
    assert!(
        Setup::open(
            target("w"),
            "E".into(),
            "env".into(),
            vec![
                (
                    declaration("token", Kind::Secret, true),
                    SavedValue::Missing
                ),
                (
                    declaration("token", Kind::Secret, true),
                    SavedValue::Missing
                )
            ],
            Origin::RequestValues
        )
        .is_err()
    );
    assert!(
        setup(SavedValue::Missing, true)
            .edit("unknown", private("value"))
            .is_err()
    );
}
#[test]
fn target_disappearance_closes_sheet_clears_drafts_and_rejects_completion() {
    let mut s = setup(SavedValue::Missing, true);
    let mut svc = ValueService::default();
    s.edit("token", private("synthetic private draft")).unwrap();
    s.save(1, &mut svc).unwrap();
    assert_eq!(
        s.invalidate(),
        "Extension was changed, so the dialog closed."
    );
    assert!(s.closed());
    assert!(!format!("{s:?}").contains("synthetic private draft"));
    assert_eq!(s.complete(&target("w"), 1, Ok(())), Err(Error::Stale));
}
#[test]
fn every_service_code_has_only_specified_informative_copy() {
    for (code, expected) in [
        (
            ServiceCode::Offline,
            "Check your internet connection and try again.",
        ),
        (ServiceCode::NotFound, "It may have been moved or deleted."),
        (ServiceCode::TooLarge, ""),
        (ServiceCode::InvalidProject, ""),
        (
            ServiceCode::PermissionDenied,
            "Check its sharing and permissions in Finder.",
        ),
        (ServiceCode::DiskFull, "Free some space and try again."),
        (ServiceCode::InvalidInput, "The file may be damaged."),
        (ServiceCode::InvalidData, "The file may be damaged."),
        (
            ServiceCode::Unavailable,
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            ServiceCode::Unknown,
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    ] {
        assert_eq!(code.informative(), expected);
    }
}

#[test]
fn cancel_while_saving_closes_sheet_and_late_completion_cannot_reopen_it() {
    let mut s = setup(SavedValue::SecretSet, true);
    let mut svc = ValueService::default();
    s.save(1, &mut svc).unwrap();
    assert!(s.view().can_cancel);
    s.cancel().unwrap();
    assert!(s.closed());
    assert_eq!(s.complete(&target("w"), 1, Ok(())), Err(Error::Stale));
    // Already admitted service work remains explicit; Cancel is not a rollback
    // of a previously accepted value write (tagged behavior).
    assert_eq!(svc.saves.len(), 1);
}

#[test]
fn significant_whitespace_is_a_present_value_and_unreadable_warning_clears_live() {
    let mut s = setup(SavedValue::Unreadable, true);
    s.edit("token", private(" \n ")).unwrap();
    assert!(s.view().can_save);
    assert_eq!(s.view().rows[1].warning, None);
    assert_eq!(s.field_value("token").unwrap(), " \n ");
}

#[test]
fn local_removal_commands_address_ids_and_archive_instead_of_erasing() {
    for permanent in [false, true] {
        let (mut a, mut s, mut h) = (Authority::default(), Services::default(), Host::default());
        let prompt = if permanent {
            Prompt::DeleteLocal {
                project: "local-id".into(),
                name: "Panel".into(),
            }
        } else {
            Prompt::TrashLocal {
                project: "local-id".into(),
                name: "Panel".into(),
            }
        };
        let id = enqueue(&mut a, &mut s, &mut h, "w", prompt);
        a.answer(
            &target("w"),
            id,
            if permanent {
                Action::Delete
            } else {
                Action::MoveToTrash
            },
            &mut s,
            &mut h,
        )
        .unwrap();
        assert!(
            matches!(s.commands.last(), Some(Command::ArchiveLocal { project, .. } | Command::MoveLocalToTrash { project, .. }) if project == "local-id")
        );
    }
}

#[test]
fn agent_state_and_concealed_clipboard_contracts_have_exact_labels() {
    assert_eq!(
        alert(full()).inspection().agent_state_label(),
        Some("Awaiting answer")
    );
    assert_eq!(alert(workspace()).inspection().agent_state_label(), None);
    assert_eq!(CONCEALED_PASTEBOARD_TYPE, "org.nspasteboard.ConcealedType");
}

#[test]
fn bounded_bridge_is_nonblocking_and_fifo_for_authority_work() {
    let (mut svc, inbox) = bridge::channel(1);
    svc.enqueue(Command::RunDecision {
        target: target("w"),
        request: 1,
        run: "run-id".into(),
        granted: false,
    })
    .unwrap();
    assert_eq!(
        svc.enqueue(Command::RunDecision {
            target: target("w"),
            request: 2,
            run: "other".into(),
            granted: true
        }),
        Err(ServiceCode::Unavailable)
    );
    assert!(matches!(
        inbox.try_recv().unwrap(),
        bridge::Work::Authority(Command::RunDecision {
            request: 1,
            granted: false,
            ..
        })
    ));
}

#[test]
fn disconnected_bridge_fails_closed_and_preserves_secret_save_payload() {
    let (mut svc, inbox) = bridge::channel(1);
    drop(inbox);
    let save = Save {
        request: 1,
        target: target("w"),
        env_key: "env".into(),
        writes: vec![Write::Set {
            key: "token".into(),
            kind: Kind::Secret,
            value: private("synthetic exact secret"),
        }],
    };
    let (code, returned) = svc.enqueue_save(save).unwrap_err();
    assert_eq!(code, ServiceCode::Unavailable);
    match &returned.writes[0] {
        Write::Set { value, .. } => assert_eq!(value.expose(), "synthetic exact secret"),
        _ => panic!("payload preserved"),
    }
    assert!(!format!("{returned:?}").contains("synthetic exact secret"));
}

#[test]
fn bridge_remove_and_explicit_reveal_keep_provenance_and_target_generation() {
    let (mut svc, inbox) = bridge::channel(2);
    svc.enqueue_remove(target("w"), 1, "env-id".into(), "token".into())
        .unwrap();
    svc.reveal(
        target("w"),
        2,
        "env-id".into(),
        "token".into(),
        "API Key".into(),
    )
    .unwrap();
    assert!(
        matches!(inbox.try_recv().unwrap(), bridge::Work::Remove { target:t, request:1, env_key, key } if t == target("w") && env_key == "env-id" && key == "token")
    );
    assert!(
        matches!(inbox.try_recv().unwrap(), bridge::Work::Reveal { request:2, env_key, label, .. } if env_key == "env-id" && label == "API Key")
    );
}

#[test]
fn reveal_completion_and_setup_host_inspection_never_expose_secret_contents() {
    let reply = bridge::Completion::Revealed {
        target: target("w"),
        request: 1,
        label: "API Key".into(),
        result: Ok(private("synthetic private result")),
    };
    assert!(!format!("{reply:?}").contains("synthetic private result"));
    #[derive(Default)]
    struct InspectSetupHost {
        sheet: Option<SheetView>,
    }
    impl bridge::SetupHost for InspectSetupHost {
        fn present_sheet(&mut self, view: SheetView) {
            self.sheet = Some(view);
        }
        fn update_sheet(&mut self, view: SheetView) {
            self.sheet = Some(view);
        }
        fn close_sheet(&mut self, _: &Target) {
            self.sheet = None;
        }
    }
    use bridge::SetupHost;
    let mut host = InspectSetupHost::default();
    let mut s = setup(SavedValue::Missing, true);
    host.present_sheet(s.view());
    s.edit("token", private("synthetic private draft")).unwrap();
    host.update_sheet(s.view());
    assert!(!format!("{:?}", host.sheet).contains("synthetic private draft"));
    host.close_sheet(&target("w"));
    assert!(host.sheet.is_none());
}
