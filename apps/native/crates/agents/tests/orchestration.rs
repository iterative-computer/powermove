use pm_agents::{
    Access, Provider, RunResult,
    orchestration::{self, Barrier, Command, Delegate, Ids, Scope, Selection},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};
#[derive(Default)]
struct Host {
    archives: BTreeMap<PathBuf, Value>,
    fail_save: bool,
    fail_provider: bool,
    saves: usize,
}
impl orchestration::Host for Host {
    fn load(&mut self, path: &Path) -> io::Result<Option<Value>> {
        Ok(self.archives.get(path).cloned())
    }
    fn save(&mut self, path: &Path, expected: Option<&Value>, value: &Value) -> io::Result<()> {
        if self.archives.get(path) != expected {
            return Err(io::Error::other("stale scope"));
        }
        if self.fail_save {
            return Err(io::Error::other("disk full"));
        }
        self.saves += 1;
        self.archives.insert(path.into(), value.clone());
        Ok(())
    }
    fn select(&mut self, _: &Value, _: Option<&Value>) -> io::Result<Selection> {
        if self.fail_provider {
            return Err(io::Error::other("provider unavailable"));
        }
        Ok(Selection {
            provider: Provider::Claude,
            model: Some("fixture-model".into()),
            reasoning_effort: Some("high".into()),
        })
    }
}
fn root() -> Value {
    json!({"id":"root","threadId":"root-thread","projectId":"project","provider":"chatgpt","mode":"autonomous","access":"project","prompt":"Original authored intent","planning":true,"images":["image"],"attachments":[{"id":"a"}],"future":{"preserve":true}})
}
fn input(key: &str) -> Delegate {
    Delegate {
        task: " Complete child brief ".into(),
        title: None,
        role: Some("test".into()),
        target: None,
        client_request_id: Some(key.into()),
    }
}
fn ids(n: usize) -> Ids {
    Ids {
        task: format!("task-{n}"),
        thread: format!("thread-{n}"),
        run: format!("run-{n}"),
    }
}
fn success(text: &str) -> RunResult {
    RunResult::success(text.into(), Access::Project)
}
#[test]
fn reservation_is_durable_before_start_and_retries_do_not_duplicate_work() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    let (task, command) = scope
        .delegate(&mut host, "root", input("one"), ids(1), 1)
        .unwrap();
    assert_eq!(host.archives[scope.path()], *scope.raw());
    let Some(Command::Start {
        request,
        root: original,
        role,
    }) = command
    else {
        panic!("missing start")
    };
    assert_eq!(request["prompt"], "Complete child brief");
    for field in [
        "projectId",
        "mode",
        "access",
        "planning",
        "images",
        "attachments",
        "future",
    ] {
        assert_eq!(request[field], root()[field]);
    }
    assert_eq!(request["provider"], "claude");
    assert_eq!(original, root());
    assert_eq!(role.as_deref(), Some("test"));
    let (retried, command) = scope
        .delegate(&mut host, "root", input("one"), ids(2), 2)
        .unwrap();
    assert_eq!(retried, task);
    assert!(command.is_none());
    assert_eq!(scope.raw()["records"].as_array().unwrap().len(), 1);
    let mut changed = input("one");
    changed.task = "different task".into();
    assert!(
        scope
            .delegate(&mut host, "root", changed, ids(2), 2)
            .is_err()
    );
}
#[test]
fn failed_storage_or_provider_discovery_never_reserves_or_starts_a_task() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    let before = scope.raw().clone();
    host.fail_provider = true;
    assert!(
        scope
            .delegate(&mut host, "root", input("one"), ids(1), 1)
            .is_err()
    );
    assert_eq!(scope.raw(), &before);
    assert!(host.archives.is_empty());
    host.fail_provider = false;
    host.fail_save = true;
    assert!(
        scope
            .delegate(&mut host, "root", input("one"), ids(1), 1)
            .is_err()
    );
    assert_eq!(scope.raw(), &before);
    assert!(host.archives.is_empty());
    host.fail_save = false;
    assert!(
        scope
            .delegate(&mut host, "root", input("one"), ids(1), 1)
            .unwrap()
            .1
            .is_some()
    );
}
#[test]
fn concurrent_results_wait_for_all_children_and_delivery_is_recoverable_until_ack() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    scope
        .delegate(&mut host, "root", input("one"), ids(1), 1)
        .unwrap();
    scope
        .delegate(&mut host, "root", input("two"), ids(2), 2)
        .unwrap();
    assert_eq!(
        scope.after_turn(&mut host, "root").unwrap(),
        Barrier::Waiting
    );
    scope
        .finish(&mut host, "run-2", &success("second"), 3)
        .unwrap();
    assert_eq!(
        scope.after_turn(&mut host, "root").unwrap(),
        Barrier::Waiting
    );
    scope
        .finish(&mut host, "run-1", &success("first"), 4)
        .unwrap();
    let pending = scope.after_turn(&mut host, "root").unwrap();
    let Barrier::Continue(Command::Continue {
        delivery,
        run,
        tasks,
        ..
    }) = &pending
    else {
        panic!("missing continuation")
    };
    assert_eq!(run, "root");
    assert_eq!(
        tasks
            .iter()
            .map(|t| t["summary"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    let mut reopened = Scope::load(&mut host, root(), 5).unwrap();
    assert_eq!(reopened.after_turn(&mut host, "root").unwrap(), pending);
    host.fail_save = true;
    assert!(
        reopened
            .acknowledge_delivery(&mut host, "root", delivery)
            .is_err()
    );
    assert_eq!(reopened.after_turn(&mut host, "root").unwrap(), pending);
    host.fail_save = false;
    reopened
        .acknowledge_delivery(&mut host, "root", delivery)
        .unwrap();
    reopened
        .acknowledge_delivery(&mut host, "root", delivery)
        .unwrap();
    assert_eq!(
        reopened.after_turn(&mut host, "root").unwrap(),
        Barrier::Complete
    );
}
#[test]
fn terminal_status_acknowledges_but_active_status_and_failed_finish_do_not() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    scope
        .delegate(&mut host, "root", input("one"), ids(1), 1)
        .unwrap();
    assert_eq!(
        scope.status(&mut host, "root", "task-1").unwrap()["status"],
        "queued"
    );
    host.fail_save = true;
    assert!(
        scope
            .finish(&mut host, "run-1", &success("full result"), 2)
            .is_err()
    );
    assert_eq!(scope.raw()["records"][0]["task"]["status"], "queued");
    host.fail_save = false;
    scope
        .finish(&mut host, "run-1", &success("full result"), 2)
        .unwrap();
    assert_eq!(
        scope.status(&mut host, "root", "task-1").unwrap()["summary"],
        "full result"
    );
    assert_eq!(
        scope.after_turn(&mut host, "root").unwrap(),
        Barrier::Complete
    );
    assert!(
        scope
            .delegate(&mut host, "run-1", input("invalid"), ids(2), 3)
            .is_err()
    );
}
#[test]
fn cancellation_is_owned_drains_descendants_and_never_stops_ordinary_threads() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    scope
        .delegate(&mut host, "root", input("one"), ids(1), 1)
        .unwrap();
    scope
        .delegate(&mut host, "run-1", input("two"), ids(2), 2)
        .unwrap();
    assert!(scope.cancel(&mut host, "run-2", "task-1").is_err());
    assert_eq!(
        scope.cancel(&mut host, "root", "task-1").unwrap(),
        vec![
            Command::Cancel {
                run: "run-1".into()
            },
            Command::Cancel {
                run: "run-2".into()
            }
        ]
    );
    assert!(
        scope
            .cancel(&mut host, "root", "task-1")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        scope
            .finish(&mut host, "run-1", &success("raced"), 3)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    scope
        .finish(&mut host, "run-2", &success("raced"), 3)
        .unwrap();
    scope
        .finish(&mut host, "run-1", &success("raced"), 4)
        .unwrap();
    assert_eq!(
        scope.status(&mut host, "root", "task-1").unwrap()["status"],
        "cancelled"
    );
    assert_eq!(
        scope.after_turn(&mut host, "root").unwrap(),
        Barrier::Complete
    );
}
#[test]
fn reopen_preserves_unknown_fields_and_recovers_interrupted_reservations_without_restarting() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    scope
        .delegate(&mut host, "root", input("one"), ids(1), 1)
        .unwrap();
    let path = scope.path().to_owned();
    host.archives.get_mut(&path).unwrap()["opaque"] = json!({"keep":[1,2,3]});
    let mut reopened = Scope::load(&mut host, root(), 2).unwrap();
    assert_eq!(
        reopened.raw()["records"][0]["task"]["status"],
        "interrupted"
    );
    assert_eq!(reopened.raw()["opaque"], json!({"keep":[1,2,3]}));
    let (task, command) = reopened
        .delegate(&mut host, "root", input("one"), ids(2), 3)
        .unwrap();
    assert_eq!(task["status"], "interrupted");
    assert!(command.is_none());
    let old = host.archives[&path].clone();
    host.archives.get_mut(&path).unwrap()["version"] = json!(99);
    assert!(Scope::load(&mut host, root(), 3).is_err());
    assert_eq!(host.archives[&path]["version"], 99);
    host.archives.insert(path, old);
}
#[test]
fn parallel_and_depth_limits_are_enforced_without_consuming_reservations() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    for n in 1..=8 {
        scope
            .delegate(&mut host, "root", input(&format!("k{n}")), ids(n), 1)
            .unwrap();
    }
    assert!(
        scope
            .delegate(&mut host, "root", input("overflow"), ids(9), 1)
            .is_err()
    );
    assert_eq!(scope.raw()["records"].as_array().unwrap().len(), 8);
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    for n in 1..=4 {
        let parent = if n == 1 {
            "root".into()
        } else {
            format!("run-{}", n - 1)
        };
        scope
            .delegate(&mut host, &parent, input(&format!("k{n}")), ids(n), 1)
            .unwrap();
    }
    assert!(
        scope
            .delegate(&mut host, "run-4", input("too-deep"), ids(5), 1)
            .is_err()
    );
}

#[test]
fn frozen_task_reservation_request_archive_and_fingerprint_are_identical() {
    use sha2::{Digest, Sha256};
    use std::{fs, process::Command as Process};
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture: Value =
        serde_json::from_slice(&fs::read(base.join("fixtures/tasks.json")).unwrap()).unwrap();
    assert_eq!(
        fixture["baseline"],
        "07ed5fa17ee640659cb075f9cad1af2095011d60"
    );
    for source in fixture["sources"].as_array().unwrap() {
        let output = Process::new("git")
            .args([
                "show",
                &format!(
                    "{}:{}",
                    fixture["baseline"].as_str().unwrap(),
                    source["file"].as_str().unwrap()
                ),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            format!("{:x}", Sha256::digest(output.stdout)),
            source["sha256"]
        );
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(base.join("scripts/capture-tasks.cjs")).unwrap())
        ),
        fixture["captureSha256"]
    );
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, fixture["root"].clone(), 0).unwrap();
    let ids = Ids {
        task: fixture["ids"]["task"].as_str().unwrap().into(),
        thread: fixture["ids"]["thread"].as_str().unwrap().into(),
        run: fixture["ids"]["run"].as_str().unwrap().into(),
    };
    let (task, command) = scope
        .delegate(
            &mut host,
            "root",
            serde_json::from_value(fixture["input"].clone()).unwrap(),
            ids,
            1,
        )
        .unwrap();
    assert_eq!(task, fixture["queued"]);
    assert_eq!(scope.raw(), &fixture["archive"]);
    assert_eq!(scope.path(), Path::new(fixture["path"].as_str().unwrap()));
    let Some(Command::Start {
        request,
        root: original,
        role,
    }) = command
    else {
        panic!("missing start")
    };
    assert_eq!(request, fixture["request"]);
    assert_eq!(original, fixture["root"]);
    assert_eq!(role.as_deref(), Some("test"));
}

#[test]
fn root_stop_is_durable_before_cancel_and_survives_restart() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    scope
        .delegate(&mut host, "root", input("one"), ids(1), 1)
        .unwrap();
    host.fail_save = true;
    assert!(scope.stop(&mut host).is_err());
    assert_eq!(
        scope.after_turn(&mut host, "root").unwrap(),
        Barrier::Waiting
    );
    host.fail_save = false;
    assert_eq!(
        scope.stop(&mut host).unwrap(),
        vec![
            Command::Cancel { run: "root".into() },
            Command::Cancel {
                run: "run-1".into()
            }
        ]
    );
    assert!(scope.after_turn(&mut host, "root").is_err());
    assert!(scope.stop(&mut host).unwrap().is_empty());
    let mut reopened = Scope::load(&mut host, root(), 2).unwrap();
    assert!(
        reopened
            .delegate(&mut host, "root", input("blocked"), ids(2), 3)
            .is_err()
    );
    let mut next_root = root();
    next_root["id"] = json!("new-root");
    let mut next = Scope::load(&mut host, next_root, 3).unwrap();
    assert!(
        next.delegate(&mut host, "new-root", input("next"), ids(2), 4)
            .unwrap()
            .1
            .is_some()
    );
}
#[test]
fn child_and_watch_results_share_a_durable_barrier_and_release_only_after_ack() {
    use pm_agents::monitor::{Monitor, Run, Status};
    let monitored = |thread: &str, id: &str| Run {
        project_id: "project".into(),
        thread_id: thread.into(),
        run_id: id.into(),
        status: Status::Queued,
        provider_instance_id: Provider::Chatgpt,
        model: None,
        reasoning_effort: None,
        progress: String::new(),
        summary: None,
        started_at: 0,
        ended_at: None,
    };
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    let mut monitor = Monitor::default();
    monitor.begin(monitored("root-thread", "root"), 0).unwrap();
    monitor
        .begin(monitored("ordinary", "ordinary-run"), 0)
        .unwrap();
    let lease = monitor.watch("project", "root", "ordinary-run").unwrap();
    let leases = [lease];
    monitor
        .begin(monitored("other-root-thread", "other-root"), 0)
        .unwrap();
    let foreign = monitor
        .watch("project", "other-root", "ordinary-run")
        .unwrap();
    assert!(
        scope
            .after_turn_with_watches(&mut host, "root", &monitor, std::slice::from_ref(&foreign))
            .is_err()
    );
    scope
        .delegate(&mut host, "root", input("one"), ids(1), 1)
        .unwrap();
    scope
        .finish(&mut host, "run-1", &success("child"), 2)
        .unwrap();
    assert_eq!(
        scope
            .after_turn_with_watches(&mut host, "root", &monitor, &leases)
            .unwrap(),
        Barrier::Waiting
    );
    monitor.finish("ordinary-run", &success("watched result"), 3);
    let pending = scope
        .after_turn_with_watches(&mut host, "root", &monitor, &leases)
        .unwrap();
    let Barrier::Continue(Command::Continue {
        delivery,
        watched,
        tasks,
        ..
    }) = &pending
    else {
        panic!("missing watch continuation")
    };
    assert_eq!(watched[0]["summary"], "watched result");
    assert_eq!(tasks[0]["summary"], "child");
    assert!(
        scope
            .acknowledge_with_watches(
                &mut host,
                "root",
                delivery,
                &mut monitor,
                std::slice::from_ref(&foreign)
            )
            .is_err()
    );
    let mut reopened = Scope::load(&mut host, root(), 4).unwrap();
    // Watches are transient, but an already scheduled delivery survives shutdown.
    assert_eq!(reopened.after_turn(&mut host, "root").unwrap(), pending);
    host.fail_save = true;
    assert!(
        reopened
            .acknowledge_with_watches(&mut host, "root", delivery, &mut monitor, &leases)
            .is_err()
    );
    assert!(monitor.is_watching(&leases[0]));
    host.fail_save = false;
    reopened
        .acknowledge_with_watches(&mut host, "root", delivery, &mut monitor, &leases)
        .unwrap();
    assert!(!monitor.is_watching(&leases[0]));
    assert!(monitor.is_watching(&foreign));
    assert_eq!(
        reopened
            .after_turn_with_watches(&mut host, "root", &monitor, &leases)
            .unwrap(),
        Barrier::Complete
    );
    assert_eq!(
        monitor
            .get("project", "ordinary", Some("ordinary-run"))
            .unwrap()
            .unwrap()
            .status,
        Status::Completed
    );
}
#[test]
fn malformed_archives_and_scope_mismatches_never_publish_replacements() {
    let mut host = Host::default();
    let scope = Scope::load(&mut host, root(), 0).unwrap();
    let path = scope.path().to_owned();
    for archive in [
        json!({"version":1,"records":[],"rootThreadId":"another-thread"}),
        json!({"version":1,"records":[],"nativePendingDeliveries":{}}),
        json!({"version":1,"records":[],"nativePendingDeliveries":[{"id":"bad"}]}),
        json!({"version":1,"records":[],"nativeRootStop":false}),
        json!({"version":1,"records":[{"task":{"taskId":"unknown"}}]}),
    ] {
        host.archives.insert(path.clone(), archive.clone());
        let saves = host.saves;
        assert!(Scope::load(&mut host, root(), 1).is_err());
        assert_eq!(host.archives[&path], archive);
        assert_eq!(host.saves, saves);
    }
}

#[test]
fn invalid_target_settings_fail_before_provider_selection_or_reservation() {
    let mut host = Host::default();
    let mut scope = Scope::load(&mut host, root(), 0).unwrap();
    for target in [
        json!(null),
        json!([]),
        json!({"providerInstanceId":"unknown"}),
        json!({"model":null}),
        json!({"unknown":true}),
        json!({"options":{"reasoningEffort":"invented"}}),
    ] {
        let mut request = input("invalid");
        request.target = Some(target);
        assert!(
            scope
                .delegate(&mut host, "root", request, ids(1), 1)
                .is_err()
        );
        assert_eq!(scope.raw()["records"], json!([]));
        assert!(host.archives.is_empty());
    }
}

#[test]
fn stale_scope_cannot_overwrite_a_concurrent_reservation() {
    let mut host = Host::default();
    let mut first = Scope::load(&mut host, root(), 0).unwrap();
    let mut stale = Scope::load(&mut host, root(), 0).unwrap();
    first
        .delegate(&mut host, "root", input("first"), ids(1), 1)
        .unwrap();
    let durable = host.archives[first.path()].clone();
    assert!(
        stale
            .delegate(&mut host, "root", input("second"), ids(2), 2)
            .is_err()
    );
    assert_eq!(host.archives[first.path()], durable);
    assert_eq!(stale.raw()["records"], json!([]));
}
