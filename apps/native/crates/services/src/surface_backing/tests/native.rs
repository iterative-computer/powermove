use super::*;
use crate::surface_backing::native::{tasks::*, *};
use pm_first_launch::{Event, Outcome, Step};
use pm_promedia::{
    storage::{Location, Storage, StorageClass, StorageSettings},
    tasks::{BackgroundTasks, ProgressUnit, Task, TaskKind, TaskSpec, TaskState},
};
use std::collections::BTreeMap;

#[test]
fn migration_waiting_and_secret_consent_cannot_attach_library() {
    let mut m = Migration::default();
    let r = m.begin().unwrap();
    m.event(&r, Event::Complete).unwrap();
    assert!(!m.library_may_attach());
    m.settle(&r, Ok(Outcome::WaitingForElectron)).unwrap();
    assert_eq!(m.state, MigrationState::WaitingForElectron);
    let r = m.begin().unwrap();
    m.settle(&r, Ok(Outcome::NeedsSecretConsent { interrupted: true }))
        .unwrap();
    assert!(!m.library_may_attach());
    let consent = m.consent().unwrap();
    assert!(m.consent().is_err());
    m.settle(
        &consent,
        Ok(Outcome::NeedsSecretConsent { interrupted: true }),
    )
    .unwrap();
    let retry = m.begin().unwrap();
    assert!(
        run_migration(
            &pm_first_launch::Config {
                electron_profile: "unused".into(),
                native_profile: "unused".into()
            },
            &retry,
            None,
            |_| Ok(())
        )
        .is_err()
    );
}
#[test]
fn secret_capability_is_required_only_for_fresh_consent_request() {
    let mut m = Migration::default();
    assert!(m.consent().is_err());
    let r = m.begin().unwrap();
    m.settle(&r, Ok(Outcome::NeedsSecretConsent { interrupted: false }))
        .unwrap();
    let r = m.consent().unwrap();
    let error = run_migration(
        &pm_first_launch::Config {
            electron_profile: "unused".into(),
            native_profile: "unused".into(),
        },
        &r,
        None,
        |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.code, ServiceErrorCode::Unavailable);
}
#[test]
fn migration_pause_waits_for_durable_published_step() {
    let mut m = Migration::default();
    let r = m.begin().unwrap();
    m.request_pause().unwrap();
    assert!(
        m.event(
            &r,
            Event::Progress {
                step: Step::Checkpoint,
                done: 1,
                total: 2
            }
        )
        .unwrap()
    );
    assert_eq!(m.state, MigrationState::Running);
    assert!(!m.event(&r, Event::Published(Step::Checkpoint)).unwrap());
    m.paused(&r).unwrap();
    assert_eq!(m.state, MigrationState::Paused);
    assert!(!m.library_may_attach());
    let next = m.begin().unwrap();
    assert!(m.event(&r, Event::Complete).is_err());
    assert!(m.event(&next, Event::Started(Step::Media)).unwrap());
}
#[test]
fn migration_refuses_invalid_progress_and_keeps_failures_visible() {
    let mut m = Migration::default();
    let r = m.begin().unwrap();
    assert!(
        m.event(
            &r,
            Event::Progress {
                step: Step::Media,
                done: 4,
                total: 3
            }
        )
        .is_err()
    );
    assert_eq!(
        m.settle(&r, Err(failure())).unwrap_err().code,
        ServiceErrorCode::DiskFull
    );
    assert_eq!(m.state, MigrationState::Failed(ServiceErrorCode::DiskFull));
}
#[test]
fn migration_outcomes_keep_every_nonideal_count() {
    let mut m = Migration::default();
    let r = m.begin().unwrap();
    m.settle(
        &r,
        Ok(Outcome::Complete {
            projects: 8,
            history_refusals: 2,
            missing_projects: 3,
            blocked_owners: 4,
            proxy_only: 5,
            already_offline: 6,
        }),
    )
    .unwrap();
    assert!(m.library_may_attach());
    assert_eq!(
        m.state,
        MigrationState::Complete(MigrationSummary {
            projects: 8,
            history_refusals: 2,
            missing_projects: 3,
            blocked_owners: 4,
            proxy_only: 5,
            already_offline: 6
        })
    );
}
#[test]
fn migration_rollback_is_not_an_optimistic_capability() {
    struct Host;
    impl MigrationReviewHost for Host {
        fn inspect_retained(&mut self, _: &str) -> Result<MigrationReview> {
            Err(unavailable("missing"))
        }
        fn reveal_archive(&mut self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        Host.rollback("review").unwrap_err().code,
        ServiceErrorCode::Unavailable
    );
}
fn task(queue: &mut BackgroundTasks, total: u64) -> pm_promedia::tasks::TaskId {
    queue
        .enqueue(
            "window".into(),
            "project".into(),
            TaskSpec {
                title: "Authored title".into(),
                kind: TaskKind::Import,
                unit: ProgressUnit::Items,
                priority: 0,
                total,
            },
        )
        .unwrap()
}
#[test]
fn task_pause_cancel_remain_pending_until_worker_ack() {
    let mut q = BackgroundTasks::default();
    let id = task(&mut q, 10);
    let t = q.start_next().unwrap();
    assert!(act(&mut q, "other", id, TaskAction::Pause).is_err());
    act(&mut q, "window", id, TaskAction::Pause).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::PauseRequested);
    q.settle_control(t).unwrap();
    act(&mut q, "window", id, TaskAction::Resume).unwrap();
    let t = q.start_next().unwrap();
    act(&mut q, "window", id, TaskAction::Cancel).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::CancelRequested);
    q.settle_control(t).unwrap();
    assert_eq!(q.get(id).unwrap().state, TaskState::Cancelled);
}
#[test]
fn task_summary_unknown_total_is_indeterminate_and_windows_do_not_leak_rows() {
    let mut q = BackgroundTasks::default();
    task(&mut q, 0);
    let mut rows = TaskRows::default();
    rows.refresh(&q, "window");
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.overall_fraction(), None);
    let mut other = TaskRows::default();
    other.refresh(&q, "other");
    assert!(other.rows.is_empty());
}
#[test]
fn task_archive_failure_keeps_terminal_row_and_success_retains_diagnostics() {
    struct Fail;
    impl TaskArchive for Fail {
        fn retain(&mut self, _: &Task) -> Result<()> {
            Err(failure())
        }
    }
    let mut q = BackgroundTasks::default();
    let id = task(&mut q, 1);
    let t = q.start_next().unwrap();
    q.finish(t, Err("private synthetic diagnostic".into()))
        .unwrap();
    assert!(retire(&mut q, &mut Fail, "window", id).is_err());
    assert!(q.get(id).is_some());
    let root = tempfile::tempdir().unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let mut store = crate::storage::ServiceStorage::open(canonical.join("service")).unwrap();
    retire(&mut q, &mut store, "window", id).unwrap();
    assert!(q.get(id).is_none());
    assert_eq!(
        store
            .setting(&format!("services.task-archive.{}", id.0))
            .unwrap()
            .unwrap()["error"],
        "private synthetic diagnostic"
    );
}
#[test]
fn retired_tasks_reset_views_instead_of_leaving_ghost_rows() {
    struct Archive;
    impl TaskArchive for Archive {
        fn retain(&mut self, _: &Task) -> Result<()> {
            Ok(())
        }
    }
    let mut q = BackgroundTasks::default();
    let id = task(&mut q, 1);
    let t = q.start_next().unwrap();
    q.finish(t, Ok(())).unwrap();
    let mut rows = TaskRows::default();
    rows.refresh(&q, "window");
    retire(&mut q, &mut Archive, "window", id).unwrap();
    rows.refresh(&q, "window");
    assert!(rows.rows.is_empty());
}
#[test]
fn queue_retry_preserves_frozen_project_options_output_and_unknown_metadata() {
    let job = json!({"id":"render","status":"failed","error":"old","project":{"exact":[1,2]},"options":{"format":"mov","output":"authored"},"future":7});
    let before = records(vec![job.clone()]);
    let mut host = MemoryRecords {
        current: Some(before.clone()),
        ..Default::default()
    };
    legacy_queue_action(&mut host, &before, "render", QueueAction::Retry).unwrap();
    let after = &host.current.unwrap().entries[0];
    assert_eq!(after["status"], "pending");
    assert_eq!(after["project"], job["project"]);
    assert_eq!(after["options"], job["options"]);
    assert_eq!(after["future"], 7);
}
#[test]
fn render_queue_worker_receives_frozen_source_without_swapping_editor() {
    struct Host;
    impl RenderQueueHost for Host {
        fn render_snapshot(&mut self, id: &str, p: &Value, o: &Value) -> Result<()> {
            assert_eq!(id, "r");
            assert_eq!(p["authored"], 1);
            assert_eq!(o["output"], "x");
            Ok(())
        }
    }
    let r = records(vec![
        json!({"id":"r","status":"pending","project":{"authored":1},"options":{"output":"x"}}),
    ]);
    run_legacy_job(&mut Host, &r, "r").unwrap();
}
fn storage(root: &std::path::Path) -> Storage {
    let locations = [
        StorageClass::Originals,
        StorageClass::Proxies,
        StorageClass::Optimized,
        StorageClass::Renders,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, c)| {
        (
            c,
            Location {
                directory: root.join(format!("class{i}")),
                quota_bytes: 1000,
            },
        )
    })
    .collect();
    Storage::open(StorageSettings {
        locations,
        archive: root.join("archive"),
    })
    .unwrap()
}
#[test]
fn storage_closure_failure_or_reference_never_archives_media() {
    struct Closure;
    impl StorageClosure for Closure {
        fn referenced(&mut self, _: StorageClass, _: &str) -> Result<bool> {
            Ok(true)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let storage = storage(&canonical);
    let path = storage
        .put(StorageClass::Originals, "media", b"exact bytes")
        .unwrap();
    assert!(archive_storage(&storage, &mut Closure, StorageClass::Originals, "media").is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"exact bytes");
}
#[test]
fn storage_archive_retains_exact_bytes_and_does_not_promise_freed_space() {
    struct Closure;
    impl StorageClosure for Closure {
        fn referenced(&mut self, _: StorageClass, _: &str) -> Result<bool> {
            Ok(false)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let storage = storage(&canonical);
    storage
        .put(StorageClass::Proxies, "proxy", b"exact bytes")
        .unwrap();
    let archived = archive_storage(&storage, &mut Closure, StorageClass::Proxies, "proxy").unwrap();
    assert_eq!(std::fs::read(archived).unwrap(), b"exact bytes");
    let usage = storage_usage(&storage).unwrap();
    assert_eq!(usage[&StorageClass::Proxies].active_bytes, 0);
}
#[test]
fn storage_closure_unknown_is_not_unreferenced() {
    struct Closure;
    impl StorageClosure for Closure {
        fn referenced(&mut self, _: StorageClass, _: &str) -> Result<bool> {
            Err(unavailable("closure incomplete"))
        }
    }
    let root = tempfile::tempdir().unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let storage = storage(&canonical);
    let path = storage
        .put(StorageClass::Originals, "media", b"keep")
        .unwrap();
    assert!(archive_storage(&storage, &mut Closure, StorageClass::Originals, "media").is_err());
    assert!(path.exists());
}
#[test]
fn tasks_report_real_determinate_progress_after_worker_publication() {
    let mut q = BackgroundTasks::default();
    task(&mut q, 10);
    let t = q.start_next().unwrap();
    q.progress(t, 3, 10).unwrap();
    let mut rows = TaskRows::default();
    rows.refresh(&q, "window");
    assert_eq!(rows.overall_fraction(), Some(0.3));
    let metrics = BTreeMap::from([("active", active_ids(&rows).len())]);
    assert_eq!(metrics["active"], 1);
}
