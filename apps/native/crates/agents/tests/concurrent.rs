use pm_agents::{
    Effects, Event, codex,
    delivery::{Delivery, Queue, Surface},
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
#[test]
fn frozen_concurrent_runs_keep_admission_scope_question_ids_and_cancel_fences() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture: Value =
        serde_json::from_slice(&fs::read(base.join("fixtures/concurrent.json")).unwrap()).unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(
        fixture["baseline"],
        "07ed5fa17ee640659cb075f9cad1af2095011d60"
    );
    for source in fixture["sources"].as_array().unwrap() {
        let output = Command::new("git")
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
            Sha256::digest(fs::read(base.join("scripts/capture-concurrent.cjs")).unwrap())
        ),
        fixture["captureSha256"]
    );
    let mut c = codex::Client::default();
    let mut queue = Queue::new(1024);
    let mut actual = vec![];
    let mut expected = vec![];
    let mut inspection = vec![];
    for frame in fixture["frames"].as_array().unwrap() {
        let kind = frame["kind"].as_str().unwrap();
        let value = &frame["value"];
        let e: Effects = match kind {
            "run" => c
                .run(serde_json::from_value(value.clone()).unwrap(), 0)
                .unwrap(),
            "in" => c.receive(value.clone(), 0),
            "cancel" => c.cancel(value.as_str().unwrap(), 0).unwrap(),
            "answer" => c
                .answer(
                    value["id"].as_str().unwrap(),
                    value["itemId"].as_str().unwrap(),
                    &serde_json::from_value(value["answers"].clone()).unwrap(),
                )
                .unwrap(),
            "out" | "trace" | "progress" | "result" => {
                expected.push(frame.clone());
                continue;
            }
            other => panic!("unsupported concurrent frame {other}"),
        };
        queue.push(Surface::Codex, e).unwrap();
        while let Some(entry) = queue.pop() {
            let (kind, value) = match entry.delivery {
                Delivery::Wire(v) => ("out", v),
                Delivery::Callback(Event::Trace { run, value }) => {
                    ("trace", json!({"run":run,"value":value}))
                }
                Delivery::Callback(Event::Progress { run, text }) => {
                    ("progress", json!({"run":run,"value":text}))
                }
                Delivery::Callback(Event::Completed { run, result }) => {
                    ("result", json!({"run":run,"value":result}))
                }
                Delivery::Inspection(event) => {
                    inspection.push(event);
                    continue;
                }
                other => panic!("unexpected delivery {other:?}"),
            };
            actual.push(json!({"kind":kind,"value":value,"after":frame["after"]}));
        }
    }
    // Effects has always supplied separate wire/event collections. Compare
    // each stream exactly, including causal positions, without inventing a
    // cross-stream order for the frozen callback implementation.
    for wire in [true, false] {
        let stream = |frames: &Vec<Value>| {
            frames
                .iter()
                .filter(|f| (f["kind"] == "out") == wire)
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(stream(&actual), stream(&expected), "wire={wire}");
    }
    assert_eq!(
        inspection,
        vec![
            Event::Ready,
            Event::Session {
                run: "z-first".into(),
                id: "thread-z".into()
            },
            Event::Session {
                run: "a-second".into(),
                id: "thread-a".into()
            }
        ]
    );
}
