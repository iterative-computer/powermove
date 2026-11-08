use pm_mcp::{Automation, McpSession, catalog, transport::Rpc};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct Fixture {
    root: PathBuf,
    host: Arc<Mutex<Automation>>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-wave4"));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
        host.create_project(&json!({"name":"Synthetic conformance"}))
            .unwrap();
        Self {
            root,
            host: Arc::new(Mutex::new(host)),
        }
    }
    fn session(&self) -> McpSession {
        McpSession::new(self.host.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn all_tool_session_gates_and_parser_precedence_match_213_main_refusals() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../fixtures/error-precedence-main-c7f46a056.json"
    ))
    .unwrap();
    assert_eq!(oracle["tag"], "main");
    let f = Fixture::new();
    let mut covered = std::collections::HashSet::new();
    for test in oracle["cases"].as_array().unwrap() {
        let mut session = f.session();
        let state = test["state"].as_str().unwrap();
        if state == "closed" {
            session.close().unwrap();
        } else if state != "none" {
            assert_eq!(session.call("start_session",json!({"prompt":"Synthetic gates","access":if state=="editor" {"editor"}else{"project"},"context":if state=="app"{"app"}else{"project"}}))["isError"],false);
        }
        let tool = test["tool"].as_str().unwrap();
        let args = if test["recipe"] == "oversized-prompt" {
            json!({"prompt":"😀".repeat(200001)})
        } else {
            test["args"].clone()
        };
        assert_eq!(
            session.call(tool, args),
            json!({"isError":true,"content":[{"type":"text","text":test["error"]}]}),
            "{state} {tool}: {test}"
        );
        covered.insert(tool.to_owned());
    }
    for tool in catalog::tools() {
        assert!(covered.contains(tool["name"].as_str().unwrap()));
    }
}

#[test]
fn rpc_preserves_plain_error_content_and_transport_argument_precedence() {
    let f = Fixture::new();
    let mut rpc = Rpc::new(f.session());
    rpc.handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .unwrap();
    for (args, expected) in [
        (
            json!({}),
            json!({"isError":true,"content":[{"type":"text","text":"Call start_session before using Powermove tools."}]}),
        ),
        (
            Value::Null,
            json!({"isError":true,"content":[{"type":"text","text":"Call start_session before using Powermove tools."}]}),
        ),
    ] {
        assert_eq!(rpc.handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"apply_commands","arguments":args}})).unwrap()["result"],expected);
    }
    assert_eq!(rpc.handle(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"apply_commands","arguments":[]}})).unwrap()["error"],json!({"code":-32602,"message":"Invalid tool arguments"}));
}

#[test]
fn project_memory_inventory_matches_pinned_main_and_retains_legacy_evidence() {
    // A511/A512 remain named cutover requirements. A false tagged attribution
    // must neither silently grow the frozen catalog nor erase project data.
    let actual = std::process::Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/memory-contract.mjs"))
        .output()
        .unwrap();
    assert!(
        actual.status.success(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    let evidence: Value = serde_json::from_slice(&actual.stdout).unwrap();
    assert_eq!(evidence["toolCount"], 68);
    assert_eq!(evidence["legacyToolCount"], 67);
    assert_eq!(evidence["legacyProjectMemoryTool"], false);
    assert_eq!(evidence["projectMemoryTool"], true);
    assert_eq!(evidence["projectStateMentionsMemory"], true);
    assert_eq!(evidence["projectStateReturnsMemory"], true);
    assert!(catalog::spec("project_memory").is_some());
    assert!(!catalog::permits("inspection", "project_memory"));
}

fn body_oracle(input: Value) -> Value {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/clean-pairs.mjs"))
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
fn clean_oracle(commands: Value) -> Value {
    body_oracle(json!({"commands":commands}))
}
#[test]
fn whole_batch_cleaning_matches_tag_before_any_semantics_or_revision_admission() {
    let cases = vec![
        json!([]),
        json!([null]),
        json!([[]]),
        json!(["not JSON"]),
        json!(["{}"]),
        json!([{"type":"not-a-command"}]),
        json!([{"type":"replace_keyframes","keyframes":null}]),
        json!([{"type":"set_easing"}]),
        json!([{"type":"create_section","section":{}}]),
        json!([{"type":"update_section","sectionId":"s","layers":[]}]),
        json!([{"type":"set_property","target":"missing","value":3},{"type":"bad"}]),
        json!([{"type":"add_layer","name":"Synthetic","ignored":"x".repeat(1000001)}]),
        json!([{"type":"add_layer","name":"😀".repeat(333000)}]),
        json!([{"type":"add_layer","name":"😀".repeat(500001)}]),
        json!([{"type":"create_section","section":{"layers":[],"__proto__":{"secret":"drop"},"constructor":"drop","prototype":"drop","keep":true}}]),
        json!([{"type":"set_property","preserveHandEdits":null},{"type":"replace_keyframes","keyframes":[],"preserveHandEdits":false}]),
    ];
    for raw in cases {
        let expected = clean_oracle(raw.clone());
        match pm_mcp::commands::clean_commands(raw.as_array().unwrap()) {
            Ok(actual) => assert_eq!(json!(actual), expected["commands"]),
            Err(error) => assert_eq!(error.message, expected["error"]),
        }
    }
    let f = Fixture::new();
    let mut session = f.session();
    session
        .dispatch("start_session", &json!({"prompt":"Synthetic precedence"}))
        .unwrap();
    // Make the session stale through a second real session.
    let mut foreign = f.session();
    foreign
        .dispatch("start_session", &json!({"prompt":"Foreign edit"}))
        .unwrap();
    foreign
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"foreign","layerType":"solid"}]}),
        )
        .unwrap();
    foreign
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    let raw = json!([{"type":"set_property","target":"missing"},{"type":"bad"}]);
    let expected = clean_oracle(raw.clone());
    assert_eq!(
        session.call("apply_commands", json!({"commands":raw}))["content"][0]["text"],
        expected["error"]
    );
    // A well-formed command is still fenced by the unrefreshed revision.
    assert_eq!(
        session
            .dispatch(
                "apply_commands",
                &json!({"commands":[{"type":"add_layer","layerType":"solid"}]})
            )
            .unwrap_err()
            .code,
        "revision_conflict"
    );
}

#[test]
fn explicit_file_ingress_save_reopen_keeps_original_audio_memory_and_history() {
    use pm_kernel::{
        DocumentStore,
        project_io::{MediaEntry, MediaSource, OpenOptions, SaveOptions, SaveTarget, open_project},
    };
    let f = Fixture::new();
    // A real synthetic PCM WAV; this tests packing/retention, not decoding or
    // audible playback. The independent byte oracle is the authored source.
    let samples: Vec<i16> = (0..800)
        .map(|i| ((i as f64 * std::f64::consts::TAU * 440.0 / 8000.0).sin() * 8000.0) as i16)
        .collect();
    let data_bytes = (samples.len() * 2) as u32;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data_bytes).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_bytes.to_le_bytes());
    for sample in samples {
        wav.extend(sample.to_le_bytes());
    }
    let memory = json!({"goal":"Synthetic goal","rules":["Keep source"],"openItems":["Synthetic pending"],"decisions":[{"reason":"Synthetic authored intent"}],"unknown":{"keep":true}});
    let project = json!({"id":"file-memory-fixture","name":"Synthetic original","w":320,"h":180,"dur":1,"fps":30,"layers":[],"assets":{"tone":{"id":"tone","kind":"audio","name":"Synthetic tone","persisted":true}},"memory":memory,"unknownFutureField":{"retained":true}});
    let mut store =
        DocumentStore::new_untitled(pm_model::json::parse(&project.to_string()).unwrap());
    store.media_mut().insert(MediaEntry {
        id: "tone".into(),
        media_type: "audio/wav".into(),
        revision: MediaEntry::fresh_revision(),
        source: MediaSource::Memory(wav.clone().into()),
    });
    let original = f.root.join("original.pmv");
    let working = f.root.join("working.pmv");
    store
        .prepare_save(
            SaveTarget::SaveAs {
                path: original.clone(),
                replace: false,
            },
            SaveOptions::default(),
        )
        .unwrap()
        .run()
        .unwrap();
    drop(store);
    let original_bytes = std::fs::read(&original).unwrap();
    std::fs::copy(&original, &working).unwrap();
    let mut host =
        Automation::new(f.root.join("file-profile"), f.root.join("file-exports")).unwrap();
    assert!(
        host.open_file("bad-relative", "relative.pmv".into())
            .is_err()
    );
    host.open_file("file-memory-fixture", working.clone())
        .unwrap();
    assert_eq!(
        host.read_document("file-memory-fixture").unwrap()["memory"],
        memory
    );
    let shared = Arc::new(Mutex::new(host));
    let mut session = McpSession::new(shared.clone());
    session
        .dispatch(
            "start_session",
            &json!({"prompt":"Synthetic native ingress"}),
        )
        .unwrap();
    session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_composition","patch":{"name":"Synthetic edited"}}]}),
        )
        .unwrap();
    session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    session
        .dispatch(
            "start_session",
            &json!({"prompt":"Save synthetic working copy"}),
        )
        .unwrap();
    assert_eq!(
        session.dispatch("save_project", &json!({})).unwrap()["path"],
        json!(working)
    );
    assert_ne!(std::fs::read(&working).unwrap(), original_bytes);
    session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    drop(session);
    drop(shared);
    let opened = open_project(&working, OpenOptions::default()).unwrap();
    assert_eq!(opened.file.media.len(), 1);
    opened
        .file
        .media
        .get("tone")
        .unwrap()
        .copy_to(&f.root.join("saved-tone.wav"))
        .unwrap();
    assert_eq!(std::fs::read(f.root.join("saved-tone.wav")).unwrap(), wav);
    let store = DocumentStore::open(opened);
    assert_eq!(
        store.history().len(),
        1,
        "MCP edits retain actual native Undo history in the PMV"
    );
    let saved = pm_model::canonical::realize_project(store.snapshot().document.to_value().unwrap());
    let saved: Value = serde_json::from_str(&pm_model::json::to_canonical_string(&saved)).unwrap();
    assert_eq!(saved["memory"], memory);
    assert_eq!(saved["unknownFutureField"], project["unknownFutureField"]);
    assert_eq!(saved["name"], "Synthetic edited");
    assert_eq!(
        std::fs::read(original).unwrap(),
        original_bytes,
        "original archive must remain byte-identical"
    );
    // Opening the saved file through the same public host ingress preserves it.
    let mut reopened = Automation::new(
        f.root.join("reopened-profile"),
        f.root.join("reopened-exports"),
    )
    .unwrap();
    reopened.open_file("file-memory-fixture", working).unwrap();
    assert_eq!(
        reopened.read_document("file-memory-fixture").unwrap()["memory"],
        memory
    );
}

#[test]
fn selection_body_errors_deduplication_and_extra_fields_match_tag() {
    let f = Fixture::new();
    let mut session = f.session();
    session
        .dispatch("start_session", &json!({"prompt":"Synthetic selection"}))
        .unwrap();
    session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"a","layerType":"solid"}]}),
        )
        .unwrap();
    for args in [
        json!({}),
        json!({"layerIds":null}),
        json!({"layerIds":[1]}),
        json!({"layerIds":["missing"]}),
        json!({"layerIds":["a","missing"]}),
        json!({"layerIds":["a","a"],"extra":true}),
        json!({"layerIds":[],"add":"not-true"}),
    ] {
        let host = f.host.lock().unwrap();
        let id = host.active.as_ref().unwrap();
        let before = host.selection(id).unwrap();
        let layers = host.read_document(id).unwrap()["layers"].clone();
        drop(host);
        let expected = body_oracle(
            json!({"tool":"select_layers","arguments":args,"selection":before,"layers":layers}),
        );
        match session.dispatch("select_layers", &args) {
            Ok(value) => assert_eq!(value, expected["result"]),
            Err(error) => {
                assert_eq!(error.message, expected["error"]);
                let host = f.host.lock().unwrap();
                assert_eq!(
                    host.selection(host.active.as_ref().unwrap()).unwrap(),
                    before
                );
            }
        }
    }
}

#[test]
fn import_and_thread_tools_refuse_in_a_host_that_has_not_installed_them() {
    let f = Fixture::new();
    let mut session = f.session();
    session
        .dispatch("start_session", &json!({"prompt":"Host-backed refusals"}))
        .unwrap();
    for (name, arguments) in [
        ("import_media", json!({"path":"/synthetic/a.png"})),
        (
            "import_media_to_timeline",
            json!({"path":"/synthetic/a.png"}),
        ),
        (
            "replace_media",
            json!({"assetId":"a","path":"/synthetic/a.png"}),
        ),
        ("thread_list", json!({})),
        ("thread_read", json!({"threadId":"t"})),
    ] {
        assert!(catalog::HOST_BACKED.contains(&name));
        let error = session.dispatch(name, &arguments).unwrap_err();
        assert_eq!(error.code, "not_available_in_this_build", "{name}");
    }
}
#[test]
fn remaining_26_bodies_remain_explicit_blockers_without_mutating_the_project() {
    let evidence: Value =
        serde_json::from_str(include_str!("../fixtures/unavailable-wave4.json")).unwrap();
    let pending: std::collections::HashSet<_> = catalog::tools()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .filter(|n| !catalog::IMPLEMENTED.contains(n))
        .collect();
    assert_eq!(pending.len(), 26);
    let historical = evidence["cases"].as_array().unwrap();
    assert_eq!(historical.len(), 35);
    // Studio and durable project-memory bodies have their own admitted regressions.
    assert!(catalog::IMPLEMENTED.contains(&"edit_3d"));
    assert!(catalog::IMPLEMENTED.contains(&"project_memory"));
    let cases = historical
        .iter()
        .filter(|c| pending.contains(c["tool"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), pending.len());
    let f = Fixture::new();
    let mut session = f.session();
    session
        .dispatch(
            "start_session",
            &json!({"prompt":"Explicit backing failures"}),
        )
        .unwrap();
    let host = f.host.lock().unwrap();
    let id = host.active.as_ref().unwrap().clone();
    let before = host.read_document(&id).unwrap();
    let history = host.store(&id).unwrap().history().len();
    drop(host);
    let mut covered = std::collections::HashSet::new();
    for case in cases {
        let name = case["tool"].as_str().unwrap();
        assert!(pending.contains(name));
        assert!(covered.insert(name));
        catalog::validate(name, &case["arguments"])
            .unwrap_or_else(|e| panic!("invalid synthetic case {name}: {e}"));
        let error = session
            .dispatch(name, &case["arguments"])
            .expect_err("an absent capability cannot report success");
        assert_eq!(error.code, "not_available_in_this_build", "{name}: {error}");
        assert!(
            error.backing.is_some(),
            "missing backing must remain inspectable"
        );
        let host = f.host.lock().unwrap();
        assert_eq!(host.read_document(&id).unwrap(), before, "{name}");
        assert_eq!(host.store(&id).unwrap().history().len(), history, "{name}");
    }
}

#[test]
fn replaced_document_retires_run_archives_authored_stage_and_allows_new_session() {
    let f = Fixture::new();
    let mut s = f.session();
    let opened = s
        .dispatch("start_session", &json!({"prompt":"Old document"}))
        .unwrap();
    let workspace = PathBuf::from(opened["workspace"].as_str().unwrap());
    std::fs::write(
        workspace.join("extensions/authored.js"),
        b"Synthetic authored source",
    )
    .unwrap();
    s.dispatch(
        "apply_commands",
        &json!({"commands":[{"type":"add_layer","id":"retained","layerType":"solid"}]}),
    )
    .unwrap();
    let old = f.host.lock().unwrap().active.clone().unwrap();
    f.host
        .lock()
        .unwrap()
        .create_project(&json!({"name":"Replacement document"}))
        .unwrap();
    assert_eq!(
        s.call("get_project_state", json!({}))["content"][0]["text"],
        "Call start_session before using Powermove tools."
    );
    let archive = f
        .root
        .join("mcp-archives")
        .join(workspace.file_name().unwrap())
        .join("workspace/extensions/authored.js");
    assert_eq!(
        std::fs::read(archive).unwrap(),
        b"Synthetic authored source"
    );
    s.dispatch(
        "start_session",
        &json!({"prompt":"Recovered on same external connection"}),
    )
    .unwrap();
    assert_eq!(
        s.dispatch("get_project_state", &json!({})).unwrap()["composition"]["name"],
        "Replacement document"
    );
    s.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    f.host.lock().unwrap().open_project(&old).unwrap();
    assert_eq!(
        f.host.lock().unwrap().read_document(&old).unwrap()["layers"][0]["id"],
        "retained"
    );
}
