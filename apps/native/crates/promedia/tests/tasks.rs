use pm_promedia::tasks::*;
fn add(q: &mut BackgroundTasks, window: &str, priority: i32) -> TaskId {
    q.enqueue(window.into(), "p".into(), TaskKind::Proxy, priority, 10)
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
        q.get(id).unwrap().error.as_deref(),
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
        q.enqueue("".into(), "p".into(), TaskKind::Render, 0, 0),
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
        .enqueue("a".into(), "p".into(), TaskKind::Import, 0, 0)
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
