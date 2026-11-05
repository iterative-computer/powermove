use pm_mcp::{Automation, McpSession, memory};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
fn oracle(input: Value) -> Value {
    let mut child = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/memory-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn normalized(value: Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(
        &pm_model::json::parse(&value.to_string()).unwrap(),
    ))
    .unwrap()
}
fn entry(id: &str, kind: &str, updated: u32) -> Value {
    json!({"id":id,"kind":kind,"text":format!("Synthetic {kind}"),"author":"agent","createdAt":0,"updatedAt":updated})
}
#[test]
fn native_memory_sanitization_index_and_list_match_current_main() {
    let entries = json!([entry("goal","goal",1),entry("rule","constraint",2),entry("open","open",3),entry("decision","decision",4),{"id":"empty","kind":"goal","text":" "},{"id":"bad","kind":"unknown","text":"Invalid"},null,entry("goal","decision",6)]);
    let expected = oracle(json!({"operation":"sanitize","entries":entries,"now":1000}));
    let sanitized = normalized(json!(memory::sanitize(&entries, 1000.0)));
    assert_eq!(sanitized, expected["result"]);
    assert_eq!(
        memory::index(&sanitized),
        oracle(json!({"operation":"index","entries":sanitized}))["result"]
    );
    for args in [
        json!({}),
        json!({"kind":"open"}),
        json!({"limit":0}),
        json!({"limit":1}),
        json!({"kind":"decision","limit":1000}),
    ] {
        let expected = oracle(json!({"operation":"list","entries":sanitized,"arguments":args}));
        assert_eq!(
            memory::list(&json!({"memory":sanitized}), &args).unwrap()["memory"],
            expected["result"]
        );
    }
}
#[test]
fn every_memory_write_operation_and_validation_precedence_matches_main() {
    let mut entries = vec![
        entry("goal", "goal", 1),
        entry("constraint", "constraint", 2),
        entry("open", "open", 3),
    ];
    for args in [
        json!({"operation":"add","kind":"goal","text":"Updated goal","resolved":true}),
        json!({"operation":"add","kind":"decision","text":"Decision","why":"Reason"}),
        json!({"operation":"update","id":"open","text":"Open edited","why":"Reason","resolved":true}),
        json!({"operation":"update","id":"open","why":"","resolved":false}),
        json!({"operation":"remove","id":"constraint"}),
    ] {
        let expected = oracle(
            json!({"operation":"apply","entries":entries,"arguments":args,"now":1000,"id":"new"}),
        );
        let plan = memory::apply(&entries, &args, 1000.0, "new", "agent").unwrap();
        assert_eq!(
            normalized(json!({"entries":plan.entries,"entry":plan.entry})),
            expected["result"]
        );
        entries = plan.entries;
    }
    for args in [
        json!({"operation":"remove","id":"missing"}),
        json!({"operation":"update","id":"missing"}),
        json!({"operation":"update","id":"open","kind":"goal"}),
        json!({"operation":"add","kind":"bad","text":""}),
        json!({"operation":"add","kind":"decision","text":" "}),
        json!({"operation":"update","id":"open","text":" "}),
    ] {
        let expected = oracle(
            json!({"operation":"apply","entries":entries,"arguments":args,"now":1000,"id":"new"}),
        );
        assert_eq!(
            memory::apply(&entries, &args, 1000.0, "new", "agent")
                .err()
                .unwrap()
                .message,
            expected["error"]
        );
    }
}
#[test]
fn over_capacity_notes_retire_finished_history_before_standing_guidance() {
    let mut entries = (0..99)
        .map(|i| entry(&format!("e{i}"), "decision", i))
        .collect::<Vec<_>>();
    entries[0]["kind"] = json!("constraint");
    entries[80]["resolved"] = json!(true);
    entries.push(entry("goal", "goal", 100));
    let args = json!({"operation":"add","kind":"feedback","text":"Synthetic feedback"});
    let plan = memory::apply(&entries, &args, 1000.0, "new", "agent").unwrap();
    let expected = oracle(
        json!({"operation":"apply","entries":entries,"arguments":args,"now":1000,"id":"new"}),
    );
    assert_eq!(
        normalized(json!({"entries":plan.entries,"entry":plan.entry})),
        expected["result"]
    );
    assert_eq!(plan.retired, vec![entries[80].clone()]);
    assert_eq!(entries.len(), 100, "planning never mutates authored input");
}
#[test]
fn disabled_memory_refuses_before_operation_and_never_exposes_notes() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("main-memory-disabled"));
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    {
        let mut host = Automation::new(root.join("profile"), root.join("exports")).unwrap();
        let project = json!({"id":"memory-fixture","name":"Synthetic","w":320,"h":180,"layers":[],"memoryEnabled":false,"memory":[entry("private","goal",1)]});
        let file = root.join("source.pmv");
        std::fs::write(&file, project.to_string()).unwrap();
        host.open_file("memory-fixture", file).unwrap();
        let shared = Arc::new(Mutex::new(host));
        let mut session = McpSession::new(shared.clone());
        let started = session
            .dispatch("start_session", &json!({"prompt":"Memory disabled"}))
            .unwrap();
        let inputs: Value = serde_json::from_slice(
            &std::fs::read(
                PathBuf::from(started["workspace"].as_str().unwrap())
                    .join("inputs/powermove-project.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            inputs.get("memory").is_none(),
            "disabled notes must not leak through generated agent input files"
        );
        let state = session.dispatch("get_project_state", &json!({})).unwrap();
        assert_eq!(state["memory"], json!({"enabled":false}));
        for args in [
            json!({"operation":"list"}),
            json!({"operation":"bad"}),
            json!({"operation":"add","kind":"bad"}),
        ] {
            assert_eq!(
                session
                    .dispatch("project_memory", &args)
                    .unwrap_err()
                    .message,
                memory::DISABLED
            )
        }
        assert_eq!(
            shared
                .lock()
                .unwrap()
                .read_document("memory-fixture")
                .unwrap()["memory"][0]["id"],
            "private"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn composition_restore_keeps_current_memory_and_agent_access_setting() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("memory-restore"));
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    {
        let mut host = Automation::new(root.join("profile"), root.join("exports")).unwrap();
        let notes = json!([entry("retained", "decision", 5)]);
        let project = json!({"id":"memory-restore","name":"Current","w":320,"h":180,"layers":[],"memory":notes,"memoryEnabled":true});
        let path = root.join("source.pmv");
        std::fs::write(&path, project.to_string()).unwrap();
        host.open_file("memory-restore", path).unwrap();
        let mut snapshot = host.read_document("memory-restore").unwrap();
        snapshot["name"] = json!("Restored composition");
        snapshot["memory"] = json!([]);
        snapshot["memoryEnabled"] = json!(false);
        let revision = host.revision("memory-restore").unwrap();
        host.restore("memory-restore", snapshot, revision, 55)
            .unwrap();
        let after = host.read_document("memory-restore").unwrap();
        assert_eq!(after["memory"], notes);
        assert_eq!(after["memoryEnabled"], true);
        assert_eq!(after["name"], "Restored composition");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn disabled_note_redaction_is_recursive_preserves_other_fields_and_never_mutates_source() {
    let project = json!({"memoryEnabled":false,"memory":[{"text":"Private"}],"other":{"keep":true},"comps":{"child":{"memory":[{"text":"Child private"}],"other":7}}});
    let original = project.clone();
    let generated = memory::agent_snapshot(&project);
    assert!(generated.get("memory").is_none());
    assert!(generated["comps"]["child"].get("memory").is_none());
    assert!(generated["comps"]["child"].get("comps").is_none());
    assert_eq!(generated["other"], project["other"]);
    assert_eq!(generated["comps"]["child"]["other"], 7);
    assert_eq!(project, original);
}
