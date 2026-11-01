use pm_agents::{
    Access, ApprovalMode, Effects, Event, RunResult, approvals::Authority, claude, codex,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::Path,
};
fn absorb(e: Effects, out: &mut VecDeque<Value>, events: &mut VecDeque<(String, Value)>) {
    out.extend(e.outgoing);
    for ev in e.events {
        match ev {
            Event::Trace { value, .. } => events.push_back(("trace".into(), value)),
            Event::Progress { text, .. } => events.push_back(("progress".into(), json!(text))),
            Event::Completed { result, .. } => {
                events.push_back(("result".into(), serde_json::to_value(result).unwrap()))
            }
            _ => {}
        }
    }
}
fn replay(name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(format!("{name}.json"));
    let fixture: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let mut codex = codex::Client::default();
    let mut exec = pm_agents::codex_exec::Client::new(
        "oracle-run".into(),
        Authority::new(Access::Project, ApprovalMode::Auto, false).unwrap(),
        0,
    );
    let mut claude = claude::Client::new(
        "oracle-run".into(),
        Authority::new(Access::Editor, ApprovalMode::Auto, false).unwrap(),
        0,
    );
    let mut out = VecDeque::new();
    let mut events = VecDeque::new();
    for (i, f) in fixture["frames"].as_array().unwrap().iter().enumerate() {
        let v = &f["value"];
        let kind = f["kind"].as_str().unwrap();
        let e = match kind {
            "run" => codex
                .run(serde_json::from_value(v.clone()).unwrap(), 0)
                .unwrap(),
            "in" => {
                if name == "codex-exec" {
                    exec.receive(v.clone())
                } else if name.starts_with("codex") {
                    codex.receive(v.clone(), 0)
                } else {
                    claude.receive(v.clone())
                }
            }
            "answer" => {
                let answers: BTreeMap<String, Vec<String>> =
                    serde_json::from_value(v["answers"].clone()).unwrap();
                if name.starts_with("codex") {
                    codex
                        .answer(
                            v["id"].as_str().unwrap(),
                            v["itemId"].as_str().unwrap(),
                            &answers,
                        )
                        .unwrap()
                } else {
                    claude
                        .answer(v["itemId"].as_str().unwrap(), &answers)
                        .unwrap()
                }
            }
            "steer" => codex
                .steer(
                    v["id"].as_str().unwrap(),
                    v["prompt"].as_str().unwrap(),
                    &[],
                    0,
                )
                .unwrap(),
            "cancel" => codex.cancel(v.as_str().unwrap(), 0).unwrap(),
            "out" => {
                assert_eq!(
                    out.pop_front().as_ref(),
                    Some(v),
                    "{name} frame {i}: wire shape differs"
                );
                continue;
            }
            "trace" | "progress" | "result" => {
                assert_eq!(
                    events.pop_front(),
                    Some((kind.into(), v.clone())),
                    "{name} frame {i}: event differs"
                );
                continue;
            }
            _ => panic!("Unknown fixture frame {kind}"),
        };
        absorb(e, &mut out, &mut events);
    }
    assert!(out.is_empty(), "{name}: extra outgoing {out:?}");
    if name == "claude-stream" {
        assert_eq!(
            events.pop_front(),
            Some((
                "result".into(),
                serde_json::to_value(RunResult::success(
                    fixture["output"].to_string(),
                    Access::Editor
                ))
                .unwrap()
            ))
        );
    }
    assert!(events.is_empty(), "{name}: extra events {events:?}");
}
#[test]
fn codex_happy() {
    replay("codex-happy");
}
#[test]
fn codex_questions() {
    replay("codex-question");
}
#[test]
fn codex_steer() {
    replay("codex-steer");
}
#[test]
fn codex_cancel() {
    replay("codex-cancel");
}
#[test]
fn codex_compaction() {
    replay("codex-compact");
}
#[test]
fn codex_async_question() {
    replay("codex-async-question");
}
#[test]
fn claude_stdio_control() {
    replay("claude-control");
}
#[test]
fn claude_shipped_fake_runtime_stream() {
    replay("claude-stream");
}

#[test]
fn codex_shipped_exec_transcript() {
    replay("codex-exec");
}

#[test]
fn codex_tool_catalog() {
    replay("codex-tools");
}
#[test]
fn codex_retry_notifications() {
    replay("codex-retry");
}
