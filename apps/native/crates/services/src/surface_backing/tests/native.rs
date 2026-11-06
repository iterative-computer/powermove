use super::*;
use crate::surface_backing::native::{colour::*, tasks::*};
use pm_promedia::{
    scopes::ScopeConfig,
    storage::{Location, Storage, StorageClass, StorageSettings},
    tasks::{BackgroundTasks, ProgressUnit, Task, TaskKind, TaskSpec, TaskState},
};
use std::collections::BTreeMap;

fn frame(n: i64) -> FrameIdentity {
    FrameIdentity {
        session: session(1),
        revision: 2,
        frame: n,
        colour_pipeline: "bt2020-linear".into(),
    }
}
#[test]
fn colour_scopes_run_real_worker_without_clamping_hdr() {
    let mut a = ColourAnalysis::default();
    let r = a.begin(
        frame(1),
        AnalysisStage::WorkingLinear,
        ScopeConfig::default(),
    );
    let output = compute(&r, AnalysisStage::WorkingLinear, &[[2., -0.2, 0.5]], 1, 1).unwrap();
    assert_eq!(output.histogram[0].above, 1);
    assert_eq!(output.histogram[1].below, 1);
    a.settle(&r, &frame(1), Ok(output)).unwrap();
    assert!(matches!(a.state, AnalysisState::Ready(_)));
}
#[test]
fn scope_analysis_rejects_wrong_stage_invalid_pixels_and_dimensions() {
    let mut a = ColourAnalysis::default();
    let r = a.begin(
        frame(1),
        AnalysisStage::SourceCodeValues,
        ScopeConfig::default(),
    );
    assert!(compute(&r, AnalysisStage::DisplayEncoded, &[[0.; 3]], 1, 1).is_err());
    assert!(compute(&r, AnalysisStage::SourceCodeValues, &[[f64::NAN; 3]], 1, 1).is_err());
    assert!(compute(&r, AnalysisStage::SourceCodeValues, &[], 1, 1).is_err());
}
#[test]
fn stale_scope_jobs_cannot_replace_newer_frame_or_pending_result() {
    let mut a = ColourAnalysis::default();
    let old = a.begin(
        frame(1),
        AnalysisStage::WorkingLinear,
        ScopeConfig::default(),
    );
    let new = a.begin(
        frame(2),
        AnalysisStage::WorkingLinear,
        ScopeConfig::default(),
    );
    assert!(a.settle(&old, &frame(2), Err(failure())).is_err());
    assert_eq!(a.state, AnalysisState::Pending);
    assert!(a.settle(&new, &frame(3), Err(failure())).is_err());
    assert_eq!(a.state, AnalysisState::Empty);
}
#[test]
fn lut_inspection_uses_real_parser_and_does_not_apply_to_document() {
    let lut = parse_lut("LUT_1D_SIZE 2\n0 0 0\n2 3 4\n").unwrap();
    assert_eq!(inspect_lut(&lut, [0.5; 3]).unwrap(), [1., 1.5, 2.]);
    assert!(parse_lut("LUT_3D_SIZE 2\n0 0 0").is_err());
    assert!(inspect_lut(&lut, [f64::NAN, 0., 0.]).is_err());
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
