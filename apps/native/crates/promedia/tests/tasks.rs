use pm_promedia::tasks::*;
fn add(q: &mut BackgroundTasks, window: &str, priority: i32) -> TaskId {
    q.enqueue(
        window.into(),
        "p".into(),
        TaskSpec {
            title: "Clip".into(),
            kind: TaskKind::Proxy,
            unit: ProgressUnit::Frames,
            priority,
            total: 10,
        },
    )
    .unwrap()
}
#[test]
fn priority_then_fifo_and_window_events() {
    let mut q = BackgroundTasks::default();
    let a = add(&mut q, "a", 0);
    let b = add(&mut q, "b", 5);
    let c = add(&mut q, "a", 5);
    assert_eq!(q.start_next().unwrap().id, b);
    assert_eq!(q.start_next().unwrap().id, c);
    assert_eq!(q.start_next().unwrap().id, a);
    assert!(q.start_next().is_none());
    let events = q.events("a", 0);
    assert_eq!(events.events.len(), 4);
    assert!(!events.reset);
    assert!(events.events.iter().all(|e| e.task.window == "a"));
    assert!(q.events("a", events.cursor).events.is_empty());
}
#[test]
fn priority_change_affects_pending_order() {
    let mut q = BackgroundTasks::default();
    let a = add(&mut q, "a", 0);
    let _b = add(&mut q, "a", 1);
    q.priority(a, 3).unwrap();
    assert_eq!(q.start_next().unwrap().id, a);
}
#[test]
fn running_pause_requires_settlement_and_fences_old_ticket() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    let t = q.start_next().unwrap();
    q.progress(t, 3, 10).unwrap();
    q.pause(id).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::PauseRequested);
    assert!(q.resume(id).is_err());
    assert!(q.finish(t, Ok(())).is_err());
    q.settle_control(t).unwrap();
    q.resume(id).unwrap();
    let fresh = q.start_next().unwrap();
    assert_eq!(q.progress(t, 5, 10), Err(TaskError::StaleTicket));
    q.progress(fresh, 5, 10).unwrap();
    q.finish(fresh, Ok(())).unwrap();
    assert_eq!(q.get(id).unwrap().completed, 10);
}
#[test]
fn cancel_stays_visible_until_codec_releases() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    let t = q.start_next().unwrap();
    q.cancel(id).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::CancelRequested);
    assert!(q.finish(t, Ok(())).is_err());
    q.settle_control(t).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::Cancelled);
    assert!(q.progress(t, 1, 10).is_err());
}
#[test]
fn queued_pause_resume_and_cancel_without_worker() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    q.pause(id).unwrap();
    assert!(q.start_next().is_none());
    q.resume(id).unwrap();
    q.cancel(id).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::Cancelled);
    assert!(q.cancel(id).is_err());
}
#[test]
fn cancellation_supersedes_pause() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    let t = q.start_next().unwrap();
    q.pause(id).unwrap();
    q.cancel(id).unwrap();
    q.settle_control(t).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::Cancelled);
}
#[test]
fn progress_validation_is_atomic() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    let t = q.start_next().unwrap();
    q.progress(t, 4, 10).unwrap();
    let before = q.get(id).unwrap().clone();
    for (done, total) in [(3, 10), (11, 10), (5, 20)] {
        assert_eq!(q.progress(t, done, total), Err(TaskError::InvalidProgress));
        assert_eq!(q.get(id), Some(&before));
    }
}
#[test]
fn failure_is_terminal_and_inspectable() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    let t = q.start_next().unwrap();
    q.finish(t, Err("decoder unavailable".into())).unwrap();
    assert_eq!(
        q.get(id).unwrap().diagnostic_for_log(),
        Some("decoder unavailable")
    );
    assert!(q.priority(id, 1).is_err());
    assert!(q.pause(id).is_err());
}
#[test]
fn retired_tasks_reset_window_without_ghosts() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    q.cancel(id).unwrap();
    let cursor = q.events("a", 0).cursor;
    q.retire(id).unwrap();
    let e = q.events("a", cursor);
    assert!(e.reset);
    assert!(e.snapshot.is_empty());
    assert!(e.events.is_empty());
}
#[test]
fn event_compaction_requests_snapshot_and_bounds_backlog() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "a", 0);
    for p in 0..1100 {
        q.priority(id, p).unwrap();
    }
    let e = q.events("a", 0);
    assert!(e.reset);
    assert_eq!(e.snapshot.len(), 1);
    assert_eq!(e.snapshot[0].priority, 1099);
    assert!(q.events("a", e.cursor + 1).reset);
}
#[test]
fn invalid_identity_unknown_ids_and_active_retirement() {
    let mut q = BackgroundTasks::default();
    assert_eq!(
        q.enqueue(
            "".into(),
            "p".into(),
            TaskSpec {
                title: "Composition".into(),
                kind: TaskKind::Render,
                unit: ProgressUnit::Frames,
                priority: 0,
                total: 0
            }
        ),
        Err(TaskError::EmptyIdentity)
    );
    assert_eq!(q.pause(TaskId(9)), Err(TaskError::Unknown));
    let id = add(&mut q, "a", 0);
    assert_eq!(q.retire(id), Err(TaskError::InvalidTransition));
}
#[test]
fn unknown_total_can_become_known_and_task_roundtrips() {
    let mut q = BackgroundTasks::default();
    let id = q
        .enqueue(
            "a".into(),
            "p".into(),
            TaskSpec {
                title: "Clip".into(),
                kind: TaskKind::Import,
                unit: ProgressUnit::Items,
                priority: 0,
                total: 0,
            },
        )
        .unwrap();
    let t = q.start_next().unwrap();
    q.progress(t, 2, 4).unwrap();
    let task = q.get(id).unwrap();
    let json = serde_json::to_vec(task).unwrap();
    assert_eq!(serde_json::from_slice::<Task>(&json).unwrap(), *task);
}

#[test]
fn queue_recreation_fences_identical_id_and_generation() {
    let mut a = BackgroundTasks::default();
    let id = add(&mut a, "a", 0);
    let old = a.start_next().unwrap();
    let mut b = BackgroundTasks::default();
    assert_eq!(add(&mut b, "a", 0), id);
    let fresh = b.start_next().unwrap();
    assert_eq!(b.progress(old, 1, 10), Err(TaskError::StaleTicket));
    b.progress(fresh, 1, 10).unwrap();
}

#[test]
fn reviewed_kind_copy_and_symbols_are_exact() {
    for (kind, label, symbol) in [
        (TaskKind::Proxy, "Creating Proxy Media", "film.stack"),
        (
            TaskKind::Optimized,
            "Creating Optimized Media",
            "film.stack.fill",
        ),
        (TaskKind::Render, "Rendering", "square.stack.3d.down.right"),
        (TaskKind::Import, "Importing", "square.and.arrow.down"),
        (TaskKind::Export, "Exporting", "square.and.arrow.up"),
        (TaskKind::Purge, "Freeing Disk Space", "trash"),
    ] {
        assert_eq!(kind.label(), label);
        assert_eq!(kind.symbol(), symbol);
    }
}

#[test]
fn authored_title_and_units_survive_events_snapshot_and_serialization() {
    for unit in [
        ProgressUnit::Frames,
        ProgressUnit::Bytes,
        ProgressUnit::Items,
    ] {
        let mut q = BackgroundTasks::default();
        let id = q
            .enqueue(
                "window".into(),
                "project-id".into(),
                TaskSpec {
                    title: "Composition α".into(),
                    kind: TaskKind::Render,
                    unit,
                    priority: 9,
                    total: 300,
                },
            )
            .unwrap();
        let ticket = q.start_next().unwrap();
        q.progress(ticket, 128, 300).unwrap();
        let task = q.get(id).unwrap();
        assert_eq!(task.presentation().title, "Composition α");
        assert!(!format!("{:?}", task.presentation()).contains("priority"));
        assert_eq!(
            task.presentation().progress,
            TaskProgress::Determinate {
                completed: 128,
                total: 300,
                unit: Some(unit)
            }
        );
        assert_eq!(q.snapshot("window"), vec![task.clone()]);
        assert_eq!(&q.events("window", 0).events.last().unwrap().task, task);
        assert_eq!(
            serde_json::from_slice::<Task>(&serde_json::to_vec(task).unwrap()).unwrap(),
            *task
        );
    }
}

#[test]
fn missing_title_refuses_admission_without_publishing() {
    let mut q = BackgroundTasks::default();
    assert_eq!(
        q.enqueue(
            "w".into(),
            "p".into(),
            TaskSpec {
                title: String::new(),
                kind: TaskKind::Import,
                unit: ProgressUnit::Items,
                priority: 0,
                total: 0,
            }
        ),
        Err(TaskError::EmptyIdentity)
    );
    assert_eq!(q.events("w", 0).cursor, 0);
    assert!(q.snapshot("w").is_empty());
}

#[test]
fn unknown_total_stays_indeterminate_with_progress_until_total_is_known() {
    let mut q = BackgroundTasks::default();
    let id = q
        .enqueue(
            "w".into(),
            "p".into(),
            TaskSpec {
                title: "Clip".into(),
                kind: TaskKind::Import,
                unit: ProgressUnit::Bytes,
                priority: 0,
                total: 0,
            },
        )
        .unwrap();
    let ticket = q.start_next().unwrap();
    q.progress(ticket, 128, 0).unwrap();
    assert_eq!(
        q.get(id).unwrap().presentation().progress,
        TaskProgress::Indeterminate {
            completed: 128,
            unit: Some(ProgressUnit::Bytes)
        }
    );
    let before = q.get(id).unwrap().clone();
    assert_eq!(q.progress(ticket, 127, 0), Err(TaskError::InvalidProgress));
    assert_eq!(q.get(id), Some(&before));
    q.progress(ticket, 128, 300).unwrap();
    assert_eq!(
        q.get(id).unwrap().presentation().progress,
        TaskProgress::Determinate {
            completed: 128,
            total: 300,
            unit: Some(ProgressUnit::Bytes)
        }
    );
    assert_eq!(q.progress(ticket, 129, 0), Err(TaskError::InvalidProgress));
}

#[test]
fn reviewed_transitions_disable_controls_and_paused_offers_resume() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "w", 0);
    let ticket = q.start_next().unwrap();
    q.pause(id).unwrap();
    let pausing = q.get(id).unwrap().presentation();
    assert_eq!(pausing.transition_label, Some("Pausing…"));
    assert!(!pausing.pause_resume_enabled);
    assert!(!pausing.cancel_enabled);
    q.settle_control(ticket).unwrap();
    let paused = q.get(id).unwrap().presentation();
    assert_eq!(paused.pause_resume_label, Some("Resume"));
    assert!(paused.pause_resume_enabled);
    assert!(paused.cancel_enabled);
    assert_eq!(paused.transition_label, None);
    q.resume(id).unwrap();
    let ticket = q.start_next().unwrap();
    assert_eq!(
        q.get(id).unwrap().presentation().pause_resume_label,
        Some("Pause")
    );
    q.cancel(id).unwrap();
    let cancelling = q.get(id).unwrap().presentation();
    assert_eq!(cancelling.transition_label, Some("Cancelling…"));
    assert!(!cancelling.pause_resume_enabled);
    assert!(!cancelling.cancel_enabled);
    q.settle_control(ticket).unwrap();
    let cancelled = q.get(id).unwrap().presentation();
    assert_eq!(cancelled.transition_label, None);
    assert_eq!(cancelled.pause_resume_label, None);
    assert!(!cancelled.cancel_enabled);
}

#[test]
fn typed_failure_keeps_diagnostics_out_of_ui_and_uses_only_approved_copy() {
    for code in [
        TaskFailureCode::SourceOffline,
        TaskFailureCode::DiskFull,
        TaskFailureCode::CodecUnsupported,
        TaskFailureCode::Cancelled,
        TaskFailureCode::Unknown,
    ] {
        let mut q = BackgroundTasks::default();
        let id = add(&mut q, "w", 0);
        let ticket = q.start_next().unwrap();
        q.finish(
            ticket,
            Err(TaskFailure {
                code,
                diagnostic: "worker-only diagnostic".into(),
            }),
        )
        .unwrap();
        let task = q.get(id).unwrap();
        assert_eq!(task.diagnostic_for_log(), Some("worker-only diagnostic"));
        let view = task.presentation();
        assert_eq!(view.error_code, Some(code));
        assert_eq!(
            view.error_message,
            match code {
                TaskFailureCode::SourceOffline =>
                    Some("The source media is offline. Reconnect it and try again."),
                TaskFailureCode::DiskFull =>
                    Some("Not enough disk space. Free some space and try again."),
                TaskFailureCode::CodecUnsupported => Some("This media’s format isn’t supported."),
                TaskFailureCode::Unknown => Some("The task couldn’t be completed. Try again."),
                TaskFailureCode::Cancelled => None,
            }
        );
        assert!(!format!("{view:?}").contains("worker-only diagnostic"));
        assert!(!view.pause_resume_enabled);
        assert!(!view.cancel_enabled);
        assert_eq!(
            serde_json::from_slice::<Task>(&serde_json::to_vec(task).unwrap()).unwrap(),
            *task
        );
    }
}

#[test]
fn legacy_task_record_retains_log_without_treating_it_as_ui_copy() {
    let mut q = BackgroundTasks::default();
    let id = add(&mut q, "w", 0);
    let ticket = q.start_next().unwrap();
    q.finish(ticket, Err("legacy diagnostic".into())).unwrap();
    let mut json = serde_json::to_value(q.get(id).unwrap()).unwrap();
    let record = json.as_object_mut().unwrap();
    record.remove("title");
    record.remove("unit");
    record.remove("error_code");
    let legacy: Task = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.diagnostic_for_log(), Some("legacy diagnostic"));
    assert_eq!(
        legacy.presentation().error_code,
        Some(TaskFailureCode::Unknown)
    );
    assert_eq!(
        legacy.presentation().error_message,
        Some("The task couldn’t be completed. Try again.")
    );
    assert_eq!(legacy.presentation().title, "");
    assert_eq!(legacy.unit, None);
}

#[test]
fn completing_indeterminate_work_preserves_observed_counter() {
    let mut q = BackgroundTasks::default();
    let id = q
        .enqueue(
            "w".into(),
            "p".into(),
            TaskSpec {
                title: "Clip".into(),
                kind: TaskKind::Import,
                unit: ProgressUnit::Items,
                priority: 0,
                total: 0,
            },
        )
        .unwrap();
    let ticket = q.start_next().unwrap();
    q.progress(ticket, 3, 0).unwrap();
    q.finish(ticket, Ok(())).unwrap();
    assert_eq!(q.get(id).unwrap().completed, 3);
    assert_eq!(
        q.get(id).unwrap().presentation().progress,
        TaskProgress::Indeterminate {
            completed: 3,
            unit: Some(ProgressUnit::Items)
        }
    );
}
