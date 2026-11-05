mod support;
use pm_agents::{
    Access, ApprovalMode, Event, RunResult,
    approvals::Authority,
    claude, codex,
    transport::{self, Action, Machine, ProcessSpec},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::TryRecvError,
    thread,
    time::{Duration, Instant},
};
use support::Temp;
fn spec(t: &Temp, name: &str) -> ProcessSpec {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    ProcessSpec {
        executable: "/usr/bin/python3".into(),
        args: vec![
            base.join("fixtures/fake-runtime.py")
                .to_str()
                .unwrap()
                .into(),
            base.join(format!("fixtures/{name}.json"))
                .to_str()
                .unwrap()
                .into(),
        ],
        cwd: t.0.clone(),
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
    }
}
fn run() -> codex::Run {
    serde_json::from_value(json!({"id":"oracle-run","cwd":"/workspace/agent","prompt":"Make a progressive blur effect","schema":{"type":"object"},"model":"gpt-6.1-sol","reasoningEffort":"ultra","imagePaths":[],"config":{"mcp_servers":{}},"resumeThread":null})).unwrap()
}
fn replay(name: &str) {
    let t = Temp::new();
    let h = transport::spawn(
        spec(&t, name),
        if name.starts_with("codex") {
            Machine::Codex(codex::Client::default())
        } else {
            Machine::Claude(claude::Client::new(
                "oracle-run".into(),
                Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
                0,
            ))
        },
        vec![],
    );
    if name.starts_with("codex") {
        h.send(Action::Run(Box::new(run()))).ok().unwrap();
    }
    let end = Instant::now() + Duration::from_secs(5);
    let mut result = None;
    let mut stopped = false;
    let mut steered = false;
    while Instant::now() < end {
        match h.try_event() {
            Ok(Event::Session { .. }) if name == "codex-steer" && !steered => {
                steered = true;
                h.send(Action::Steer {
                    run: "oracle-run".into(),
                    prompt: "Make the edge softer".into(),
                    images: vec![],
                })
                .ok()
                .unwrap();
            }
            Ok(Event::Trace { value, .. }) if value["kind"] == "question" => {
                let (item, answers) = if name == "codex-question" {
                    (
                        "q1",
                        BTreeMap::from([("style".into(), vec!["Soft".into()])]),
                    )
                } else if name == "claude-control" {
                    (
                        "ask-1",
                        BTreeMap::from([("ask-1-0".into(), vec!["Blue".into(), "Green".into()])]),
                    )
                } else {
                    continue;
                };
                h.send(Action::Answer {
                    run: "oracle-run".into(),
                    item: item.into(),
                    answers,
                })
                .ok()
                .unwrap();
            }
            Ok(Event::Completed { result: r, .. }) => result = Some(r),
            Ok(Event::Stopped(_)) => {
                stopped = true;
                break;
            }
            Ok(_) => {}
            Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            Err(TryRecvError::Disconnected) => break,
        }
    }
    assert!(stopped, "{name}: worker did not stop");
    assert!(
        matches!(result, Some(RunResult::Success { .. })),
        "{name}: {result:?}"
    );
}
#[test]
fn app_server_real_pipes_match_ts_transcript() {
    replay("codex-happy");
}
#[test]
fn app_server_held_question_roundtrip_over_real_pipes() {
    replay("codex-question");
}
#[test]
fn app_server_steering_roundtrip_over_real_pipes() {
    replay("codex-steer");
}
#[test]
fn claude_shipped_stream_over_real_pipes() {
    replay("claude-stream");
}
#[test]
fn spawn_failure_is_reported_without_blocking_caller() {
    let t = Temp::new();
    let start = Instant::now();
    let mut s = spec(&t, "claude-stream");
    s.executable = t.0.join("missing");
    let h = transport::spawn(
        s,
        Machine::Claude(claude::Client::new(
            "r".into(),
            Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
            0,
        )),
        vec![],
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    let end = Instant::now() + Duration::from_secs(2);
    let mut failed = false;
    while Instant::now() < end {
        if let Ok(Event::Completed {
            result: RunResult::Failure { .. },
            ..
        }) = h.try_event()
        {
            failed = true;
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(failed);
}
#[test]
fn stalled_stdin_cannot_prevent_cancellation() {
    let t = Temp::new();
    let s = ProcessSpec {
        executable: "/usr/bin/python3".into(),
        args: vec!["-c".into(), "import time; time.sleep(30)".into()],
        cwd: t.0.clone(),
        env: BTreeMap::new(),
    };
    let h = transport::spawn(
        s,
        Machine::Claude(claude::Client::new(
            "r".into(),
            Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
            0,
        )),
        vec![json!({"large":"x".repeat(1024*1024)})],
    );
    h.send(Action::Cancel("r".into())).ok().unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    let mut cancelled = false;
    let mut stopped = false;
    while Instant::now() < end {
        match h.try_event() {
            Ok(Event::Completed {
                result:
                    RunResult::Failure {
                        cancelled: true, ..
                    },
                ..
            }) => cancelled = true,
            Ok(Event::Stopped(_)) => {
                stopped = true;
                break;
            }
            _ => thread::sleep(Duration::from_millis(2)),
        }
    }
    assert!(cancelled && stopped);
}

#[test]
fn checked_delivery_distinguishes_mailbox_admission_from_held_question_acceptance() {
    let t = Temp::new();
    let handle = transport::spawn(
        spec(&t, "claude-control"),
        Machine::Claude(claude::Client::new(
            "oracle-run".into(),
            Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
            0,
        )),
        vec![],
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if matches!(handle.try_event(), Ok(Event::Trace {value,..}) if value["kind"] == "question")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let missing = handle
        .send_checked(Action::Answer {
            run: "oracle-run".into(),
            item: "withdrawn".into(),
            answers: BTreeMap::new(),
        })
        .ok()
        .unwrap();
    assert!(!missing.recv_timeout(Duration::from_secs(2)).unwrap());
    let answer = Action::Answer {
        run: "oracle-run".into(),
        item: "ask-1".into(),
        answers: BTreeMap::from([("ask-1-0".into(), vec!["Blue".into(), "Green".into()])]),
    };
    let accepted = handle.send_checked(answer).ok().unwrap();
    assert!(accepted.recv_timeout(Duration::from_secs(2)).unwrap());
    let repeated = handle
        .send_checked(Action::Answer {
            run: "oracle-run".into(),
            item: "ask-1".into(),
            answers: BTreeMap::new(),
        })
        .ok()
        .unwrap();
    assert!(!repeated.recv_timeout(Duration::from_secs(2)).unwrap());
}
