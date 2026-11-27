use pm_agents::{
    Access, Event, Provider, RunResult,
    monitor::{Monitor, Run, Status},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Command};
fn run(project: &str, thread: &str, id: &str) -> Run {
    Run {
        project_id: project.into(),
        thread_id: thread.into(),
        run_id: id.into(),
        provider_instance_id: Provider::Chatgpt,
        model: None,
        reasoning_effort: None,
        status: Status::Queued,
        progress: String::new(),
        summary: None,
        started_at: 0,
        ended_at: None,
    }
}
#[test]
fn frozen_monitor_states_errors_and_watch_cycles_are_identical() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture: Value =
        serde_json::from_slice(&fs::read(base.join("fixtures/monitor.json")).unwrap()).unwrap();
    assert_eq!(
        fixture["baseline"],
        "07ed5fa17ee640659cb075f9cad1af2095011d60"
    );
    use sha2::{Digest, Sha256};
    let source = Command::new("git")
        .args([
            "show",
            &format!(
                "{}:{}",
                fixture["baseline"].as_str().unwrap(),
                fixture["source"].as_str().unwrap()
            ),
        ])
        .output()
        .unwrap();
    assert!(source.status.success());
    assert_eq!(
        format!("{:x}", Sha256::digest(source.stdout)),
        fixture["sourceSha256"]
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(base.join("scripts/capture-monitor.cjs")).unwrap())
        ),
        fixture["captureSha256"]
    );
    let mut monitor = Monitor::default();
    let mut watches = BTreeMap::new();
    assert_eq!(fixture["operations"].as_array().unwrap().len(), 28);
    for (i, frame) in fixture["operations"].as_array().unwrap().iter().enumerate() {
        let op = &frame["op"];
        let now = frame["at"].as_u64().unwrap();
        let string = |key| op[key].as_str().unwrap();
        let result: Result<Value, String> = (|| {
            Ok(match string("kind") {
                "reserve" => serde_json::to_value(monitor.reserve(
                    string("project"),
                    string("thread"),
                    string("run"),
                    now,
                )?)
                .unwrap(),
                "begin" => {
                    let r = &op["request"];
                    serde_json::to_value(monitor.begin(
                        run(
                            r["projectId"].as_str().unwrap(),
                            r["threadId"].as_str().unwrap(),
                            r["id"].as_str().unwrap(),
                        ),
                        now,
                    )?)
                    .unwrap()
                }
                "get" => serde_json::to_value(monitor.get(
                    string("project"),
                    string("thread"),
                    op["run"].as_str(),
                )?)
                .unwrap(),
                "progress" => {
                    monitor.progress(string("run"), string("text"));
                    Value::Null
                }
                "trace" => {
                    monitor.event(
                        &Event::Trace {
                            run: string("run").into(),
                            value: op["trace"].clone(),
                        },
                        now,
                    );
                    Value::Null
                }
                "finish" => {
                    monitor.finish(
                        string("run"),
                        &serde_json::from_value(op["result"].clone()).unwrap(),
                        now,
                    );
                    Value::Null
                }
                "watch" => {
                    watches.insert(
                        string("lease").to_owned(),
                        monitor.watch(string("project"), string("parent"), string("target"))?,
                    );
                    Value::Null
                }
                "release" => {
                    monitor.release(&watches[string("lease")]);
                    Value::Null
                }
                "tick" => {
                    monitor.tick(now);
                    Value::Null
                }
                "shutdown" => {
                    monitor.shutdown(now);
                    Value::Null
                }
                unknown => panic!("unsupported oracle operation {unknown}"),
            })
        })();
        let actual = match result {
            Ok(value) => json!({"ok":true,"value":value}),
            Err(error) => json!({"ok":false,"error":error}),
        };
        assert_eq!(actual, frame["result"], "monitor operation {i}: {op}");
    }
}
#[test]
fn watch_results_stay_bound_to_the_captured_run_and_release_is_idempotent() {
    let mut m = Monitor::default();
    m.begin(run("p", "a", "root"), 0).unwrap();
    m.begin(run("p", "b", "first"), 0).unwrap();
    let w = m.watch("p", "root", "first").unwrap();
    assert!(m.result(&w).is_none());
    m.begin(run("p", "b", "second"), 1).unwrap();
    m.finish(
        "second",
        &RunResult::success("second result".into(), Access::Editor),
        2,
    );
    assert!(m.result(&w).is_none());
    m.finish(
        "first",
        &RunResult::success("first result".into(), Access::Editor),
        3,
    );
    assert_eq!(
        m.result(&w).unwrap().summary.as_deref(),
        Some("first result")
    );
    m.release(&w);
    m.release(&w);
    assert!(m.result(&w).is_none());
    assert_eq!(m.get("p", "b", None).unwrap().unwrap().run_id, "second");
}
#[test]
fn watches_never_cross_projects_or_cancel_the_target_on_release() {
    let mut m = Monitor::default();
    m.begin(run("p", "a", "root"), 0).unwrap();
    m.begin(run("q", "b", "other"), 0).unwrap();
    assert!(m.watch("p", "root", "other").is_err());
    assert!(m.watch("p", "root", "root").is_err());
    m.begin(run("p", "b", "child"), 0).unwrap();
    let w = m.watch("p", "root", "child").unwrap();
    m.release(&w);
    assert_eq!(
        m.get("p", "b", Some("child")).unwrap().unwrap().status,
        Status::Running
    );
}
#[test]
fn watcher_limit_and_started_run_timeout_are_explicit() {
    let mut m = Monitor::default();
    m.begin(run("p", "a", "root"), 0).unwrap();
    m.begin(run("p", "b", "child"), 0).unwrap();
    let mut leases = vec![];
    for _ in 0..32 {
        leases.push(m.watch("p", "root", "child").unwrap());
    }
    assert!(m.watch("p", "root", "child").is_err());
    m.release(&leases[0]);
    assert!(m.watch("p", "root", "child").is_ok());
    m.tick(30_000);
    assert_eq!(
        m.get("p", "b", Some("child")).unwrap().unwrap().status,
        Status::Running
    );
}

#[test]
fn monitor_capacity_requires_durable_archival_and_never_evicts_watched_history() {
    let mut m = Monitor::default();
    for i in 0..512 {
        let id = format!("run-{i}");
        m.begin(run("p", "t", &id), 0).unwrap();
        m.finish(
            &id,
            &RunResult::success(format!("result-{i}"), Access::Editor),
            1,
        );
    }
    assert!(m.begin(run("p", "t", "overflow"), 2).is_err());
    assert!(
        m.archive_terminal("run-0", |_| Err("disk full".into()))
            .is_err()
    );
    assert!(m.get("p", "t", Some("run-0")).unwrap().is_some());
    let w = m.watch("p", "run-1", "run-0").unwrap();
    assert!(m.archive_terminal("run-0", |_| Ok(())).is_err());
    m.release(&w);
    let mut archived = None;
    m.archive_terminal("run-0", |r| {
        archived = Some(r.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(archived.unwrap().summary.as_deref(), Some("result-0"));
    m.begin(run("p", "t", "overflow"), 2).unwrap();
}
