use super::*;
use notices::*;
use panels::*;
use status::*;

fn labels(a: &Alert) -> Vec<&str> {
    a.buttons.iter().map(|b| b.label).collect()
}
fn prompt(id: &str, owner: Owner) -> Prompt {
    Prompt {
        id: id.into(),
        revision: 3,
        owner,
        alert: ask_recovery("Synthetic", false, false),
    }
}
#[test]
fn editor_failure_copy_is_exact_for_both_reasons() {
    for (reason, line) in [
        (EditorFailure::OutOfMemory, "The editor ran out of memory."),
        (
            EditorFailure::UnexpectedStop,
            "The editor stopped unexpectedly.",
        ),
    ] {
        let a = editor_failure(reason);
        assert_eq!(a.message, "Powermove needs to reload this window.");
        assert_eq!(
            a.informative,
            format!(
                "{line} Reload restores the latest local recovery. The most recent edits may be missing."
            )
        );
        assert_eq!(labels(&a), ["Reload", "Close Window"]);
        assert_eq!(a.key(Key::Return), Some(Action::Reload));
        assert_eq!(a.key(Key::Escape), None);
        assert!(!a.buttons.iter().any(|b| b.destructive));
    }
}
#[test]
fn ask_exact_copy_and_safe_keyboard() {
    for truncated in [false, true] {
        let a = ask_recovery("Launch Film", truncated, true);
        assert_eq!(
            a.message,
            "Powermove quit unexpectedly while “Launch Film” had unsaved changes."
        );
        assert_eq!(
            a.informative,
            format!(
                "Recover opens your latest edits. Open as Copy keeps the saved project unchanged and opens the edits beside it.{}",
                if truncated {
                    " The last few seconds of edits may be missing."
                } else {
                    ""
                }
            )
        );
        assert_eq!(labels(&a), ["Recover", "Open as Copy", "Discard Edits"]);
        assert_eq!(a.key(Key::Escape), Some(Action::OpenAsCopy));
        assert_eq!(a.key(Key::Return), Some(Action::Recover));
        assert!(a.buttons[2].destructive);
        assert!(!a.buttons[2].default);
    }
}
#[test]
fn unarchivable_edits_never_offer_discard() {
    assert_eq!(
        labels(&ask_recovery("Synthetic", false, false)),
        ["Recover", "Open as Copy"]
    );
}
#[test]
fn unrecoverable_preserves_saved_file_and_omits_missing_file_action() {
    let a = unrecoverable("Launch Film", true);
    assert_eq!(a.message, "“Launch Film” couldn’t be recovered.");
    assert_eq!(
        a.informative,
        "The saved project is unchanged. Details are in Window ▸ Error Log."
    );
    assert_eq!(labels(&a), ["Open Saved Version", "OK"]);
    assert_eq!(a.key(Key::Return), Some(Action::OpenSavedVersion));
    let a = unrecoverable("Launch Film", false);
    assert_eq!(labels(&a), ["OK"]);
    assert_eq!(a.key(Key::Return), Some(Action::Acknowledge));
}
#[test]
fn trusted_realm_prompt_uses_only_the_specified_actions() {
    let a = unresponsive_extension("Synthetic");
    assert_eq!(a.message, "“Synthetic” isn’t responding.");
    assert_eq!(
        a.informative,
        "You can wait, or stop it. A stopped extension stays off until you turn it on in Settings ▸ Extensions."
    );
    assert_eq!(labels(&a), ["Wait", "Stop Extension"]);
    assert_eq!(a.key(Key::Return), Some(Action::Wait));
    assert!(a.buttons[1].destructive);
}
#[test]
fn result_and_reply_context_select_every_recovery_surface() {
    assert!(recovery_surface("Synthetic", RecoveryState::None, false, false, false).is_none());
    for truncated in [false, true] {
        assert!(matches!(
            recovery_surface(
                "Synthetic",
                RecoveryState::Recovered { truncated },
                false,
                false,
                false
            ),
            Some(RecoverySurface::Banner(_))
        ));
        assert!(matches!(
            recovery_surface(
                "Synthetic",
                RecoveryState::Recovered { truncated },
                true,
                false,
                false
            ),
            Some(RecoverySurface::Alert(_))
        ));
        assert!(matches!(
            recovery_surface(
                "Synthetic",
                RecoveryState::OfferCopy { truncated },
                false,
                false,
                false
            ),
            Some(RecoverySurface::Alert(_))
        ));
    }
    assert!(matches!(
        recovery_surface(
            "Synthetic",
            RecoveryState::Unrecoverable,
            false,
            false,
            false
        ),
        Some(RecoverySurface::Alert(_))
    ));
}
#[test]
fn recovery_banner_copy_and_truncated_persistence() {
    let complete = Banner::recovered("Launch Film", false);
    assert_eq!(
        complete.text,
        "Powermove recovered “Launch Film” after an unexpected quit."
    );
    assert_eq!(complete.dismiss_after, Some(Duration::from_secs(4)));
    assert_eq!(complete.severity, Severity::Info);
    let truncated = Banner::recovered("Launch Film", true);
    assert_eq!(
        truncated.text,
        "Powermove recovered “Launch Film” after an unexpected quit. The last few seconds of edits may be missing."
    );
    assert_eq!(truncated.close_label, Some("Dismiss"));
    assert_eq!(truncated.severity, Severity::Warning);
    assert_eq!(truncated.dismiss_after, None);
}
#[test]
fn banners_queue_without_overwriting_persistent_recovery() {
    let mut q = BannerQueue::default();
    q.push(Banner::recovered("First", true), Duration::ZERO);
    q.push(Banner::recovered("Second", false), Duration::ZERO);
    q.tick(Duration::from_secs(600));
    assert!(q.visible.as_ref().unwrap().text.contains("First"));
    q.dismiss(Duration::from_secs(600));
    q.tick(Duration::from_secs(603));
    assert!(q.visible.is_some());
    q.tick(Duration::from_secs(604));
    assert!(q.visible.is_none());
    q.push(Banner::recovered("Third", true), Duration::ZERO);
    q.document_closed();
    assert!(q.visible.is_none());
}
#[test]
fn one_alert_per_owner_and_restore_order_is_fifo() {
    let owner = Owner::Window("one".into());
    let other = Owner::Window("two".into());
    let mut q = AlertQueue::default();
    let mut p = HeadlessPlatform::default();
    q.push(prompt("first", owner.clone()), &mut p).unwrap();
    q.push(prompt("second", owner.clone()), &mut p).unwrap();
    q.push(prompt("other", other), &mut p).unwrap();
    assert_eq!(p.events.len(), 2);
    assert_eq!(
        q.respond(&owner, "first", 3, Action::OpenAsCopy, &mut p),
        Ok(Action::OpenAsCopy)
    );
    q.pump(&owner, &mut p).unwrap();
    assert_eq!(q.active(&owner).unwrap().id, "second");
    assert!(matches!(&p.events[2], Presentation::Show(p) if p.id == "second"));
}
#[test]
fn delayed_callbacks_cannot_act_on_closed_or_replaced_windows() {
    let owner = Owner::Window("one".into());
    let mut q = AlertQueue::default();
    let mut p = HeadlessPlatform::default();
    q.push(prompt("first", owner.clone()), &mut p).unwrap();
    assert_eq!(
        q.respond(&owner, "first", 2, Action::Recover, &mut p),
        Err(QueueError::Stale)
    );
    assert_eq!(
        q.respond(&owner, "first", 3, Action::Discard, &mut p),
        Err(QueueError::InvalidAction)
    );
    q.close(&owner, &mut p).unwrap();
    assert_eq!(
        q.respond(&owner, "first", 3, Action::Recover, &mut p),
        Err(QueueError::Stale)
    );
    assert_eq!(
        q.push(prompt("first", owner), &mut p),
        Err(QueueError::DuplicateId)
    );
}
#[test]
fn failed_publication_does_not_consume_prompt_id() {
    let owner = Owner::Application;
    let mut q = AlertQueue::default();
    let mut p = HeadlessPlatform {
        fail_next: true,
        ..Default::default()
    };
    assert!(matches!(
        q.push(prompt("first", owner.clone()), &mut p),
        Err(QueueError::Platform(_))
    ));
    assert!(q.active(&owner).is_none());
    q.push(prompt("first", owner), &mut p).unwrap();
}
fn content(status: ViewStatus, failure: Option<Failure>) -> Content {
    PanelSnapshot {
        panel: "Tools",
        extension: "Example",
        status,
        failure,
        elapsed: Duration::from_secs(1),
        realm_prompt_open: false,
    }
    .content()
}
#[test]
fn every_extension_failure_row_has_exact_copy_and_one_fix() {
    let cases = [
        (
            ViewStatus::Starting,
            Failure::ReadyTimeout,
            "Tools Didn’t Start",
            Some("Example took too long to start."),
            Some("Try Again"),
        ),
        (
            ViewStatus::WaitingToRestart,
            Failure::Crash,
            "Tools Stopped",
            Some("It will restart in a moment."),
            Some("Restart Now"),
        ),
        (
            ViewStatus::WaitingToRestart,
            Failure::Hang,
            "Tools Stopped Responding",
            Some("Powermove stopped it and will restart it in a moment."),
            Some("Restart Now"),
        ),
        (
            ViewStatus::Disabled,
            Failure::Crash,
            "Tools Is Turned Off",
            Some("Example stopped working several times."),
            Some("Turn On"),
        ),
        (
            ViewStatus::StopFailed,
            Failure::Hang,
            "Tools Couldn’t Be Stopped",
            Some("Quit and reopen Powermove to stop it."),
            None,
        ),
        (
            ViewStatus::Starting,
            Failure::PlatformFailure,
            "Tools Can’t Run",
            Some("This Mac couldn’t start Example."),
            Some("Try Again"),
        ),
        (
            ViewStatus::Disabled,
            Failure::UserStop,
            "Tools Is Stopped",
            None,
            Some("Restart"),
        ),
    ];
    for (status, failure, title, line, button) in cases {
        let c = content(status, Some(failure));
        let Content::EmptyState {
            title: actual,
            line: actual_line,
            button: actual_button,
        } = &c
        else {
            panic!("{c:?}");
        };
        assert_eq!(actual, title);
        assert_eq!(actual_line.as_deref(), line);
        assert_eq!(actual_button.as_ref().map(|b| b.label), button);
        assert_eq!(
            c.ax_label().unwrap(),
            line.map_or_else(|| title.to_owned(), |line| format!("{title}. {line}"))
        );
        assert_eq!(c.announcement(), Some(Priority::High));
    }
}
#[test]
fn starting_delay_does_not_flash_and_stopping_is_explicit() {
    for millis in [0, 499, 500, 501] {
        let c = PanelSnapshot {
            panel: "Tools",
            extension: "Example",
            status: ViewStatus::Starting,
            failure: None,
            elapsed: Duration::from_millis(millis),
            realm_prompt_open: false,
        }
        .content();
        if millis <= 500 {
            assert_eq!(c, Content::Unchanged);
        } else {
            assert_eq!(
                c,
                Content::Loading {
                    line: "Starting Example…".into()
                }
            );
        }
    }
    for state in [ViewStatus::Stopping, ViewStatus::Sealing] {
        assert_eq!(
            content(state, None),
            Content::Loading {
                line: "Stopping Example…".into()
            }
        );
    }
    assert_eq!(content(ViewStatus::Ready, None), Content::Unchanged);
    assert_eq!(content(ViewStatus::Closed, None), Content::Hidden);
}
#[test]
fn realm_alert_suppresses_panel_buttons_even_in_disabled_state() {
    let c = PanelSnapshot {
        panel: "Tools",
        extension: "Example",
        status: ViewStatus::Disabled,
        failure: Some(Failure::Hang),
        elapsed: Duration::ZERO,
        realm_prompt_open: true,
    }
    .content();
    assert!(matches!(c, Content::EmptyState { button: None, .. }));
    assert_eq!(
        c.ax_label().unwrap(),
        "Tools Stopped Responding. Powermove stopped it and will restart it in a moment."
    );
}
#[test]
fn restart_button_is_generation_fenced() {
    let c = content(ViewStatus::WaitingToRestart, Some(Failure::Crash));
    assert!(restart_request("view", 4, 5, &c).is_none());
    assert_eq!(restart_request("view", 5, 5, &c).unwrap().generation, 5);
    assert!(restart_request("view", 5, 5, &content(ViewStatus::Stopping, None)).is_none());
}
#[test]
fn service_catalog_exact_named_copy_for_all_ten_codes() {
    for (code, message, informative) in [
        (
            ServiceCode::Offline,
            "Powermove can’t connect.",
            "Check your internet connection and try again.",
        ),
        (
            ServiceCode::NotFound,
            "“Launch Film” can’t be found.",
            "It may have been moved or deleted.",
        ),
        (
            ServiceCode::TooLarge,
            "“Launch Film” is too large to open.",
            "",
        ),
        (
            ServiceCode::InvalidProject,
            "“Launch Film” can’t be opened because it isn’t a Powermove project.",
            "",
        ),
        (
            ServiceCode::PermissionDenied,
            "You don’t have permission to open “Launch Film”.",
            "Check its sharing and permissions in Finder.",
        ),
        (
            ServiceCode::DiskFull,
            "There isn’t enough disk space.",
            "Free some space and try again.",
        ),
        (
            ServiceCode::InvalidInput,
            "“Launch Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            ServiceCode::InvalidData,
            "“Launch Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            ServiceCode::Unavailable,
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            ServiceCode::Unknown,
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    ] {
        assert_eq!(
            service_copy(code, Some("Launch Film")),
            Copy {
                message: message.into(),
                informative: informative.into()
            }
        );
        let unnamed = service_copy(code, None);
        assert!(!unnamed.message.contains("“”"));
        assert!(!unnamed.message.contains("  "));
    }
}
fn notice(action: Option<Fix>, severity: Severity) -> Notice {
    Notice {
        source: Source {
            surface: "S053",
            code: "offline".into(),
        },
        copy: service_copy(ServiceCode::Offline, None),
        severity,
        action,
    }
}
#[test]
fn notice_types_limit_fixes_and_use_only_banner_or_inline() {
    for (fix, label) in [
        (Fix::Retry, "Retry"),
        (Fix::ShowInFinder, "Show in Finder"),
        (Fix::Relink, "Relink…"),
        (Fix::TurnOff, "Turn Off"),
    ] {
        assert_eq!(fix.label(), label);
    }
    let n = notice(Some(Fix::Retry), Severity::Error);
    assert_eq!(n.priority(), Priority::High);
    assert_eq!(n.placement(true), Placement::ViewerBanner);
    assert_eq!(n.placement(false), Placement::InlineNotice);
    let b = Banner::notice(&n);
    assert_eq!(b.dismiss_after, None);
    assert_eq!(b.close_label, Some("Dismiss"));
    assert_eq!(notice(None, Severity::Info).priority(), Priority::Medium);
}
fn row(id: &str, seconds: u64) -> LogRow {
    LogRow {
        id: id.into(),
        time: std::time::UNIX_EPOCH + Duration::from_secs(seconds),
        source: Source {
            surface: "S053",
            code: "unknown".into(),
        },
        source_display: "Example".into(),
        copy: service_copy(ServiceCode::Unknown, None),
        diagnostic: "Synthetic diagnostic sentinel".into(),
    }
}
#[test]
fn error_log_newest_first_selection_copy_and_clear() {
    let mut log = ErrorLog::default();
    log.open();
    log.open();
    assert!(log.open);
    assert!(log.insert(row("new", 3)));
    assert!(log.insert(row("old", 1)));
    assert!(log.insert(row("middle", 2)));
    assert!(!log.insert(row("new", 4)));
    assert_eq!(
        log.rows().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["new", "middle", "old"]
    );
    log.select(["new".into(), "old".into(), "unknown".into()]);
    assert_eq!(log.details(), Some("Synthetic diagnostic sentinel"));
    let copied = log.copy_selected(|_| "now".into());
    assert_eq!(copied.matches("Synthetic diagnostic sentinel").count(), 2);
    assert!(!log.rows()[0].copy.message.contains("sentinel"));
    log.clear();
    assert!(log.rows().is_empty());
    assert!(log.details().is_none());
    assert_eq!(log.copy_selected(|_| "now".into()), "");
}
#[test]
fn error_log_labels_are_exact() {
    assert_eq!(LOG_TITLE, "Error Log");
    assert_eq!(LOG_COLUMNS, ["Time", "Source", "Message"]);
    assert_eq!(LOG_DETAILS, "Details");
    assert_eq!(LOG_BUTTONS, ["Copy", "Clear Log"]);
    assert_eq!(LOG_EMPTY_TITLE, "No Errors");
    assert_eq!(LOG_EMPTY_LINE, "Problems Powermove notices appear here.");
}
fn status() -> Status {
    Status {
        layers: 1,
        keyframes: 2,
        playing: false,
        width: 1920,
        height: 1080,
        rate: "30".into(),
        workspace: "Default".into(),
        extension_items: vec![],
    }
}
#[test]
fn status_exact_order_singular_plural_playback_and_ax() {
    assert_eq!(Status::AX_LABEL, "Status");
    assert_eq!(
        status().text(0),
        StatusText {
            left: vec!["1 layer".into(), "2 keyframes".into()],
            right: vec![
                "Paused".into(),
                "1920×1080".into(),
                "30 fps".into(),
                "Default".into()
            ],
            indicator_label: None
        }
    );
    let mut s = status();
    s.layers = 0;
    s.keyframes = 1;
    s.playing = true;
    s.extension_items.push(ExtensionItem::new("Example"));
    let text = s.text(3);
    assert_eq!(
        text.left,
        ["0 layers", "1 keyframe", "Editor slowed down", "Example"]
    );
    assert_eq!(text.right[0], "Playing");
    assert_eq!(
        text.indicator_label.as_deref(),
        Some("Editor slowed down, 3 issues")
    );
    assert!(!format!("{text:?}").contains("Saving"));
}
#[test]
fn extension_status_truncation_is_unicode_safe_and_has_full_tooltip() {
    for text in ["é".repeat(24), "🟠".repeat(25)] {
        let item = ExtensionItem::new(&text);
        assert!(item.text.chars().count() <= 24);
        assert_eq!(item.tooltip.is_some(), text.chars().count() > 24);
        if let Some(tooltip) = item.tooltip {
            assert_eq!(tooltip, text);
            assert!(item.text.ends_with('…'));
        }
    }
}
fn issue(target: Target, stage: Stage) -> Issue {
    Issue {
        id: "slow".into(),
        revision: 2,
        name: "Example".into(),
        milliseconds: 12.5,
        stage,
        target,
        turn_off_failed: false,
    }
}
#[test]
fn slowdown_exact_title_facts_actions_and_inline_failure() {
    assert_eq!(Slowdown::TITLE, "What’s Slowing the Editor Down");
    for (stage, text) in [
        (Stage::EditorThread, "on the editor thread"),
        (Stage::Compiling, "compiling"),
        (Stage::Gpu, "on the GPU"),
    ] {
        assert_eq!(
            issue(Target::Layer("layer".into()), stage).fact().unwrap(),
            format!("12.5 ms {text}. A 60 fps frame has 16.7 ms available.")
        );
    }
    let layer = issue(Target::Layer("layer".into()), Stage::Gpu);
    assert_eq!(layer.button(), "Select Layer");
    let mut ext = issue(Target::Extension("ext".into()), Stage::EditorThread);
    assert_eq!(ext.button(), "Turn Off");
    ext.turn_off_failed = true;
    assert_eq!(
        ext.error().unwrap(),
        "Couldn’t turn off Example. Try again in Settings ▸ Extensions."
    );
}
#[test]
fn slowdown_stale_actions_cannot_select_or_disable_new_targets() {
    let mut s = Slowdown::default();
    s.update(vec![issue(Target::Layer("layer".into()), Stage::Gpu)]);
    s.show();
    assert_eq!(
        s.activate("slow", 2),
        Some(SlowdownAction::SelectLayer("layer".into()))
    );
    assert!(s.open);
    assert!(s.activate("slow", 1).is_none());
    s.update(vec![]);
    assert!(!s.open);
    assert!(s.activate("slow", 2).is_none());
}
#[test]
fn slowdown_failure_completion_is_revision_and_target_fenced() {
    let mut s = Slowdown::default();
    s.update(vec![issue(
        Target::Extension("ext".into()),
        Stage::Compiling,
    )]);
    s.show();
    assert_eq!(
        s.activate("slow", 2),
        Some(SlowdownAction::TurnOff("ext".into()))
    );
    s.turn_off_completed("slow", 1, false);
    assert!(s.issues[0].error().is_none());
    s.turn_off_completed("slow", 2, false);
    assert!(s.issues[0].error().is_some());
    s.turn_off_completed("slow", 2, true);
    assert!(s.issues[0].error().is_none());
}
#[test]
fn invalid_timings_never_publish_fake_slowdown_facts() {
    for value in [f64::NAN, f64::INFINITY, -1.] {
        let mut i = issue(Target::Layer("layer".into()), Stage::Gpu);
        i.milliseconds = value;
        assert!(i.fact().is_none());
    }
}

#[test]
fn next_sheet_failure_keeps_accepted_action_and_can_retry() {
    let owner = Owner::Application;
    let mut q = AlertQueue::default();
    let mut host = HeadlessPlatform::default();
    q.push(prompt("first", owner.clone()), &mut host).unwrap();
    q.push(prompt("second", owner.clone()), &mut host).unwrap();
    assert_eq!(
        q.respond(&owner, "first", 3, Action::Recover, &mut host),
        Ok(Action::Recover)
    );
    host.fail_next = true;
    assert!(matches!(
        q.pump(&owner, &mut host),
        Err(QueueError::Platform(_))
    ));
    assert_eq!(
        q.respond(&owner, "second", 3, Action::Recover, &mut host),
        Err(QueueError::Stale)
    );
    q.pump(&owner, &mut host).unwrap();
    assert_eq!(
        q.respond(&owner, "second", 3, Action::OpenAsCopy, &mut host),
        Ok(Action::OpenAsCopy)
    );
}
