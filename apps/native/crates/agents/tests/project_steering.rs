mod support;

use pm_agents::{
    Access, ApprovalMode, Event, Provider, RunResult,
    launch::{self, Environment, Request},
    transport::{Action, Handle},
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    thread,
    time::{Duration, Instant},
};
use support::Temp;

fn start(temp: &Temp, scenario: &str, access: Access) -> Handle {
    let runtime = temp.0.join("fake-codex");
    fs::write(&runtime, include_str!("../fixtures/project-runtime.py")).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let env = Environment {
        profile: temp.0.join("profile"),
        vars: BTreeMap::from([
            (
                "CODEX_BINARY".into(),
                runtime.to_string_lossy().into_owned(),
            ),
            ("PM_TEST_SCENARIO".into(), scenario.into()),
        ]),
        ..Default::default()
    };
    launch::launch(
        &env,
        Request {
            run: "run".into(),
            prompt: "Keep the original brief".into(),
            provider: Provider::Chatgpt,
            access,
            approval: ApprovalMode::Supervised,
            computer_consent: access == Access::Computer,
            workspace: temp.0.clone(),
            model: None,
            effort: None,
            images: vec![],
            role: None,
            files: vec![],
            project: None,
        },
    )
    .unwrap()
}

fn await_event(handle: &Handle, matches: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = Vec::new();
    while Instant::now() < deadline {
        match handle.try_event() {
            Ok(event) if matches(&event) => return event,
            Ok(event) => observed.push(event),
            Err(std::sync::mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
    }
    panic!("Expected event did not arrive: {observed:?}");
}

fn steer_twice(handle: &Handle) {
    let receipts: Vec<_> = ["Make it blue", "Keep it short"]
        .into_iter()
        .map(|prompt| {
            handle
                .send_checked(Action::Steer {
                    run: "run".into(),
                    prompt: prompt.into(),
                    images: vec![],
                })
                .ok()
                .unwrap()
        })
        .collect();
    for receipt in receipts {
        assert!(receipt.recv_timeout(Duration::from_secs(2)).unwrap());
    }
}

fn completed(handle: &Handle) -> RunResult {
    match await_event(handle, |event| matches!(event, Event::Completed { .. })) {
        Event::Completed { run, result } => {
            assert_eq!(run, "run");
            result
        }
        _ => unreachable!(),
    }
}

fn assert_blue_reply(handle: &Handle, access: Access) {
    let RunResult::Success {
        text,
        access: actual,
        ..
    } = completed(handle)
    else {
        panic!("Expected a successful steered result");
    };
    assert_eq!(actual, access);
    assert_eq!(launch::reply_text(&text), "Short blue answer");
}

fn await_tool(handle: &Handle) {
    await_event(handle, |event| {
        matches!(event,
        Event::Trace { value, .. } if value["kind"] == "tool-start" && value["itemId"] == "work")
    });
}

#[test]
fn supervised_project_steers_live_and_routes_the_escape_to_the_card() {
    let temp = Temp::new();
    let handle = start(&temp, "project-steer", Access::Project);
    await_tool(&handle);
    steer_twice(&handle);
    let Event::Trace { value, .. } = await_event(&handle, |event| {
        matches!(event,
        Event::Trace { value, .. } if value["kind"] == "question")
    }) else {
        unreachable!()
    };
    assert_eq!(value["questions"][0]["header"], "Approval needed");
    assert!(
        value["questions"][0]["question"]
            .as_str()
            .unwrap()
            .contains("synthetic outside project")
    );
    let reply = handle
        .send_checked(Action::Answer {
            run: "run".into(),
            item: value["itemId"].as_str().unwrap().into(),
            answers: BTreeMap::from([("decision".into(), vec!["Allow once".into()])]),
        })
        .ok()
        .unwrap();
    assert!(reply.recv_timeout(Duration::from_secs(2)).unwrap());
    assert_blue_reply(&handle, Access::Project);
    assert!(!temp.0.join("exec-called").exists());
}

#[test]
fn full_access_steers_live() {
    let temp = Temp::new();
    let handle = start(&temp, "full-steer", Access::Computer);
    await_tool(&handle);
    steer_twice(&handle);
    assert_blue_reply(&handle, Access::Computer);
    assert!(!temp.0.join("exec-called").exists());
}

#[test]
fn crash_after_initialization_is_reported_without_cli_fallback() {
    let temp = Temp::new();
    let handle = start(&temp, "crash", Access::Project);
    await_tool(&handle);
    let RunResult::Failure {
        error, cancelled, ..
    } = completed(&handle)
    else {
        panic!("Expected a clean failure after the worker dies");
    };
    assert!(!cancelled);
    assert!(error.contains("Agent runtime"), "{error}");
    assert!(!temp.0.join("exec-called").exists());
}

#[test]
fn startup_failure_queues_directions_for_the_next_turn_of_the_same_run() {
    let temp = Temp::new();
    let handle = start(&temp, "startup-failure", Access::Project);
    steer_twice(&handle);
    fs::write(temp.0.join("startup-release"), "").unwrap();
    assert_blue_reply(&handle, Access::Project);
    assert!(temp.0.join("exec-called").exists());
}
