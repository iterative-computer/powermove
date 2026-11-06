use pm_mcp::{Automation, McpSession, catalog, transport::Rpc};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
struct Fixture {
    root: PathBuf,
    host: Arc<Mutex<Automation>>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-test"));
        let host = Arc::new(Mutex::new(
            Automation::new(root.clone(), root.join("exports")).unwrap(),
        ));
        Self { root, host }
    }
    fn session(&self) -> McpSession {
        McpSession::new(self.host.clone())
    }
    fn create(&self) -> String {
        self.host
            .lock()
            .unwrap()
            .create_project(&json!({"name":"Synthetic MCP fixture"}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn value(result: Value) -> Value {
    assert_eq!(result["isError"], false, "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn start(s: &mut McpSession) {
    value(s.call("start_session", json!({"prompt":"Synthetic test"})));
}
fn add(s: &mut McpSession, id: &str) {
    value(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":id,"layerType":"solid","name":"Synthetic solid"}]})));
}
fn node(script: &str, input: Option<&Value>) -> Value {
    let mut command = Command::new("node");
    command.arg(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join(script),
    );
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .expect("Node 26 required for frozen TS oracle");
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn all_schemas_equal_executed_tag_source() {
    let output = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/tag-oracle.mjs"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let current = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/main-oracle.mjs"))
        .output()
        .unwrap();
    assert!(
        current.status.success(),
        "{}",
        String::from_utf8_lossy(&current.stderr)
    );
    assert_eq!(catalog::tools().len(), 68);
    assert_eq!(
        catalog::legacy_catalog()["tools"].as_array().unwrap().len(),
        67
    );
    let mut names = std::collections::HashSet::new();
    for tool in catalog::tools() {
        assert!(names.insert(tool["name"].as_str().unwrap()));
        assert!(tool.get("outputSchema").is_none());
    }
}
#[test]
fn captured_installed_stdio_pairs_replay_with_owner_3d_contract() {
    let f = Fixture::new();
    let mut rpc = Rpc::new(f.session());
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/transport-installed.json")).unwrap();
    assert_eq!(fixture["projectToolsCaptured"], false);
    for pair in fixture["pairs"].as_array().unwrap() {
        assert_eq!(
            rpc.handle(pair["request"].clone()).unwrap(),
            if pair["request"]["method"] == "tools/list"
                && pair["response"]["result"]["tools"].is_array()
            {
                let mut expected = pair["response"].clone();
                let main: Value =
                    serde_json::from_str(include_str!("../fixtures/catalog-main-c7f46a056.json"))
                        .unwrap();
                assert_eq!(
                    expected["result"]["tools"],
                    catalog::legacy_catalog()["tools"]
                );
                let studio: Value =
                    serde_json::from_str(include_str!("../fixtures/scene3d-source-catalog.json"))
                        .unwrap();
                let mut tools = main["tools"].as_array().unwrap().clone();
                for replacement in studio["tools"].as_array().unwrap() {
                    *tools
                        .iter_mut()
                        .find(|t| t["name"] == replacement["name"])
                        .unwrap() = replacement.clone();
                }
                expected["result"]["tools"] = Value::Array(tools);
                expected
            } else {
                pair["response"].clone()
            }
        );
    }
}
#[test]
fn source_bodies_match_state_and_timeline_pages() {
    let f = Fixture::new();
    let id = f.create();
    let mut session = f.session();
    start(&mut session);
    add(&mut session, "L-fixture");
    for (tool, args) in [
        ("get_project_state", json!({})),
        ("get_project_state", json!({"indexOnly":true})),
        (
            "get_project_state",
            json!({"layerSearch":"SYNTHETIC","propertyLimit":2,"propertyOffset":1,"keyframeLimit":0}),
        ),
        ("get_project_state", json!({"layerOffset":5})),
        ("get_timeline", json!({})),
        ("get_timeline", json!({"offset":3,"limit":1})),
    ] {
        let host = f.host.lock().unwrap();
        let project = host.read_document(&id).unwrap();
        let selection = host.selection(&id).unwrap();
        drop(host);
        let expected = node(
            "source-pairs.mjs",
            Some(&json!({"project":project,"selection":selection,"tool":tool,"arguments":args})),
        );
        assert_eq!(value(session.call(tool, args)), expected, "{tool}");
    }
}
#[test]
fn guarded_session_commit_squashes_and_undo_works() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-one");
    value(s.call("apply_commands",json!({"label":"Set opacity","commands":[{"type":"set_property","target":"L-one","path":"opacity","value":42}]})));
    let finished = value(s.call("finish_session", json!({"commit":true})));
    assert_eq!(finished["changed"], true);
    assert!(finished["historyId"].is_string());
    let mut host = f.host.lock().unwrap();
    assert_eq!(host.store(&id).unwrap().history().len(), 1);
    host.kernel().sync_all();
    let pm_kernel::journal::Recovery::Recovered(recovered) = host.kernel().recover(&id, None, None)
    else {
        panic!("expected recoverable grouped history");
    };
    assert_eq!(recovered.history.len(), 1, "squash must reach the journal");
    host.undo(&id, pm_kernel::Origin::Agent, 777).unwrap();
    assert_eq!(host.read_document(&id).unwrap()["layers"], json!([]));
}
#[test]
fn rollback_is_guarded_and_undoable() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-one");
    value(s.call("rollback_changes", json!({})));
    assert_eq!(
        f.host.lock().unwrap().read_document(&id).unwrap()["layers"],
        json!([])
    );
    add(&mut s, "L-two");
    value(s.call("finish_session", json!({"commit":false})));
    assert_eq!(
        f.host.lock().unwrap().read_document(&id).unwrap()["layers"],
        json!([])
    );
}
#[test]
fn disconnect_rolls_back_and_removes_workspace() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    let opened = value(s.call("start_session", json!({"prompt":"Synthetic"})));
    let path = PathBuf::from(opened["workspace"].as_str().unwrap());
    assert!(path.exists());
    add(&mut s, "L-one");
    s.close().unwrap();
    assert!(!path.exists());
    assert_eq!(
        f.host.lock().unwrap().read_document(&id).unwrap()["layers"],
        json!([])
    );
}
#[test]
fn interleaved_edits_refuse_rollback_and_do_not_squash() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-agent");
    {
        let mut h = f.host.lock().unwrap();
        let revision = h.revision(&id).unwrap();
        h.apply_commands(
            &id,
            vec![
                pm_kernel::CompCommand::Set {
                    patch: Box::new(pm_kernel::commands::comps::CompPatch {
                        name: Some("Foreign work".into()),
                        ..Default::default()
                    }),
                }
                .into(),
            ],
            "Foreign edit",
            pm_kernel::Origin::Agent,
            revision,
            778,
        )
        .unwrap();
    }
    assert_eq!(
        diagnostic(&mut s, "rollback_changes", json!({}))["code"],
        "revision_conflict"
    );
    value(s.call("get_project_state", json!({"indexOnly":true})));
    assert_eq!(
        diagnostic(&mut s, "rollback_changes", json!({}))["code"],
        "revision_conflict"
    );
    let finish = value(s.call("finish_session", json!({"commit":true})));
    assert_eq!(finish["historyId"], Value::Null);
    let h = f.host.lock().unwrap();
    assert_eq!(h.store(&id).unwrap().history().len(), 2);
    assert_eq!(h.read_document(&id).unwrap()["name"], "Foreign work");
}
#[test]
fn batches_are_atomic_and_use_revision_guards() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    let revision = f.host.lock().unwrap().revision(&id).unwrap();
    diagnostic(
        &mut s,
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"L-a","layerType":"solid"},{"type":"set_property","target":"missing","path":"opacity","value":0}]}),
    );
    let h = f.host.lock().unwrap();
    assert_eq!(h.revision(&id).unwrap(), revision);
    assert_eq!(h.read_document(&id).unwrap()["layers"], json!([]));
    drop(h);
    let mut h = f.host.lock().unwrap();
    assert!(
        h.apply_commands(
            &id,
            vec![],
            "stale",
            pm_kernel::Origin::Mcp,
            revision + 1,
            900
        )
        .is_err()
    );
}
#[test]
fn planning_and_app_context_access_is_enforced() {
    let f = Fixture::new();
    f.create();
    let mut s = f.session();
    let opened = value(s.call(
        "start_session",
        json!({"prompt":"Inspect","access":"editor"}),
    ));
    assert!(opened.get("workspace").is_none());
    assert_eq!(
        diagnostic(&mut s, "apply_commands", json!({"commands":["{}"]}))["code"],
        "invalid_arguments"
    );
    value(s.call("get_timeline", json!({})));
    value(s.call("finish_session", json!({"commit":false})));
    diagnostic(
        &mut s,
        "start_session",
        json!({"prompt":"Bad","access":"editor","context":"app"}),
    );
    value(s.call("start_session", json!({"prompt":"App","context":"app"})));
    diagnostic(&mut s, "get_project_state", json!({}));
    assert_eq!(
        diagnostic(&mut s, "store_search", json!({"query":"test"}))["code"],
        "not_available_in_this_build"
    );
}
#[test]
fn selection_errors_never_change_selection() {
    let f = Fixture::new();
    f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-a");
    diagnostic(
        &mut s,
        "select_layers",
        json!({"layerIds":["L-a","missing"]}),
    );
    assert_eq!(
        value(s.call("select_layers", json!({"layerIds":["L-a"]})))["selection"]["layers"],
        json!(["L-a"])
    );
}
#[test]
fn create_save_reopen_library_and_pmv() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-a");
    let saved = value(s.call("save_project", json!({})));
    let path = PathBuf::from(saved["path"].as_str().unwrap());
    assert!(path.exists());
    assert!(
        pm_kernel::locks::FileLock::acquire(&pm_kernel::locks::project_lock_path(&path)).is_ok(),
        "an independent export must not become the live document's lock"
    );
    value(s.call(
        "apply_commands",
        json!({"commands":[{"type":"set_layer","target":"L-a","patch":{"name":"Second save"}}]}),
    ));
    let second = value(s.call("save_project", json!({})));
    assert_ne!(second["path"], saved["path"]);
    let opened = pm_kernel::open_project(&path, pm_kernel::OpenOptions::default()).unwrap();
    let store = pm_kernel::DocumentStore::open(opened);
    assert_eq!(store.layers()[0].id, "L-a");
    assert_eq!(store.layers()[0].name, "Synthetic solid");
    let second = PathBuf::from(second["path"].as_str().unwrap());
    let opened = pm_kernel::open_project(&second, pm_kernel::OpenOptions::default()).unwrap();
    let store = pm_kernel::DocumentStore::open(opened);
    assert_eq!(store.layers()[0].name, "Second save");
    value(s.call("finish_session", json!({"commit":true})));
    f.host.lock().unwrap().save_library(&id).unwrap();
    let lib = pm_kernel::LibraryStore::open(&f.root);
    assert!(lib.load(&id).unwrap().is_some());
}

#[test]
fn restore_validates_then_remains_undoable() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-restore");
    let mut h = f.host.lock().unwrap();
    let revision = h.revision(&id).unwrap();
    assert!(
        h.restore(&id, json!({"layers":"invalid"}), revision, 900)
            .is_err()
    );
    assert_eq!(h.revision(&id).unwrap(), revision);
    drop(h);
    value(s.call("rollback_changes", json!({})));
    let mut h = f.host.lock().unwrap();
    assert!(h.store(&id).unwrap().layers().is_empty());
    assert!(matches!(
        h.store(&id)
            .unwrap()
            .changes_since(revision)
            .unwrap()
            .last()
            .unwrap()
            .cause,
        pm_kernel::document::ChangeCause::Restore { .. }
    ));
    h.kernel().sync_all();
    let pm_kernel::journal::Recovery::Recovered(recovered) = h.kernel().recover(&id, None, None)
    else {
        panic!("expected recoverable rollback");
    };
    assert_eq!(recovered.history.len(), 2);
    assert!(
        pm_model::canonical::realize_project(recovered.document.to_value().unwrap())
            .get("layers")
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    h.undo(&id, pm_kernel::Origin::Agent, 901).unwrap();
    assert_eq!(h.store(&id).unwrap().layers()[0].id, "L-restore");
}

#[test]
fn durability_notice_does_not_hide_applied_changes_from_rollback() {
    let f = Fixture::new();
    let id = f.create();
    let journal = f.host.lock().unwrap().kernel().journal_dir(&id).unwrap();
    // Force the next rotation to fail without filling the disk or changing
    // permissions globally. The initial segment remains writable.
    let blocked = journal.join("journal-00000001.log");
    std::fs::create_dir(&blocked).unwrap();
    let mut s = f.session();
    start(&mut s);
    let mut last = Value::Null;
    for i in 0..50 {
        last = value(s.call(
            "apply_commands",
            json!({"commands":[{
                "type":"set_composition", "patch":{"name":format!("Synthetic revision {i}")}
            }]}),
        ));
    }
    assert!(last["notice"].as_str().unwrap().contains("NotDurable"));
    assert_eq!(
        f.host.lock().unwrap().read_document(&id).unwrap()["name"],
        "Synthetic revision 49"
    );
    std::fs::remove_dir(&blocked).unwrap();
    value(s.call("finish_session", json!({"commit":false})));
    assert_eq!(
        f.host.lock().unwrap().read_document(&id).unwrap()["name"],
        "Synthetic MCP fixture"
    );
}

#[test]
fn no_op_run_reports_no_document_changes() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    let revision = f.host.lock().unwrap().revision(&id).unwrap();
    value(s.call(
        "apply_commands",
        json!({"commands":[{"type":"set_composition","patch":{"name":"Synthetic MCP fixture"}}]}),
    ));
    assert_eq!(f.host.lock().unwrap().revision(&id).unwrap(), revision);
    assert_eq!(
        value(s.call("finish_session", json!({"commit":true})))["changed"],
        false
    );
}

#[test]
fn reopening_same_project_fences_the_old_run() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-preserved");
    {
        let mut host = f.host.lock().unwrap();
        host.create_project(&json!({"name":"Other synthetic project"}))
            .unwrap();
        host.open_project(&id).unwrap();
    }
    assert_eq!(
        diagnostic(&mut s, "get_project_state", json!({}))["code"],
        "invalid_arguments"
    );
    assert_eq!(
        diagnostic(&mut s, "finish_session", json!({"commit":false}))["code"],
        "invalid_arguments"
    );
    assert_eq!(
        f.host.lock().unwrap().store(&id).unwrap().layers()[0].id,
        "L-preserved"
    );
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, Wake, Waker};
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::park();
    }
}

#[test]
fn actor_save_handoff_keeps_session_generation_and_file_lock() {
    use pm_kernel::session::{SessionSource, ShellCall, ShellReply};
    let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-actor-save"));
    std::fs::create_dir_all(&root).unwrap();
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
        journal_root: Some(root.join("journals")),
        ..Default::default()
    });
    let kernel = actor.handle();
    block_on(kernel.shell(ShellCall::AttachLibrary {
        profile: root.clone(),
        format: pm_kernel::persistence::DiskFormat::V1_2,
        electron_profile: None,
    }))
    .unwrap()
    .unwrap();
    let source = || {
        SessionSource::New(
            pm_model::json::parse(
                &json!({
                    "id":"P-actor", "compId":"C-actor", "compName":"Main", "comps":{},
                    "name":"Synthetic actor save", "w":200,"h":100,"fps":30,"dur":2,
                    "work":[0,2],"bg":"#000000","layers":[],"assets":{}
                })
                .to_string(),
            )
            .unwrap(),
        )
    };
    let ShellReply::Opened { session, .. } = block_on(kernel.shell(ShellCall::Open {
        id: "P-actor".into(),
        source: source(),
        electron_profile: None,
    }))
    .unwrap()
    .unwrap() else {
        panic!("expected open session");
    };
    let host = pm_mcp::ActorAutomation::new(kernel.clone());
    let path = root.join("actor.pmv");
    assert!(matches!(
        block_on(host.save_as(session.clone(), path.clone())).unwrap(),
        ShellReply::Saved {
            recovery_copy: None,
            ..
        }
    ));
    assert!(
        pm_kernel::locks::FileLock::acquire(&pm_kernel::locks::project_lock_path(&path)).is_err()
    );
    assert!(matches!(
        block_on(host.save(session.clone())).unwrap(),
        ShellReply::Saved { .. }
    ));
    block_on(kernel.shell(ShellCall::Close(session.clone())))
        .unwrap()
        .unwrap();
    assert!(
        block_on(host.save(session)).is_err(),
        "stale session must not save"
    );
    actor.shutdown();
    assert!(pm_kernel::open_project(&path, pm_kernel::OpenOptions::default()).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn unknown_pending_and_malformed_arguments_are_structured() {
    let f = Fixture::new();
    f.create();
    let mut s = f.session();
    start(&mut s);
    assert_eq!(
        diagnostic(&mut s, "render_frames", json!({}))["code"],
        "not_available_in_this_build"
    );
    diagnostic(&mut s, "invented_tool", json!({}));
    diagnostic(&mut s, "get_timeline", json!({"limit":0}));
    diagnostic(&mut s, "get_timeline", json!({"extra":1}));
    diagnostic(&mut s, "select_layers", json!({"layerIds":["a","a"]}));
}
#[test]
fn automation_events_and_unwired_hooks_are_truthful() {
    let f = Fixture::new();
    let mut h = f.host.lock().unwrap();
    let events = h.subscribe();
    let id = h.create_project(&json!({"name":"Synthetic"})).unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let event = events.recv().unwrap();
    assert_eq!(event.project_id, id);
    assert!(event.revision > 0);
    assert!(h.list_panels().is_err());
    assert!(
        h.capture_frames(&pm_mcp::CaptureRequest {
            project_id: id.clone(),
            revision: h.revision(&id).unwrap(),
            times: vec![0.],
            width: 480
        })
        .is_err()
    );
}

#[test]
fn loopback_authentication_catalog_and_descriptor_lifetime() {
    use std::{
        io::{BufRead, BufReader},
        net::TcpStream,
        os::unix::fs::PermissionsExt,
    };
    let f = Fixture::new();
    let mut server = pm_mcp::transport::LoopbackServer::start(&f.root, f.host.clone()).unwrap();
    let file = server.descriptor_path().to_owned();
    let descriptor: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let connect =
        || TcpStream::connect(("127.0.0.1", descriptor["port"].as_u64().unwrap() as u16)).unwrap();
    let mut bad = connect();
    writeln!(
        bad,
        "{}",
        json!({"id":1,"method":"connect","token":"wrong"})
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(bad).read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert!(response.get("error").is_some());
    let mut stream = connect();
    writeln!(
        stream,
        "{}",
        json!({"id":2,"method":"connect","token":descriptor["token"]})
    )
    .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["result"]["connected"],
        true
    );
    writeln!(stream, "{}", json!({"id":3,"method":"tools/list"})).unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["result"],
        json!(catalog::tools())
    );
    drop(reader);
    drop(stream);
    server.shutdown();
    assert!(!file.exists());
}
#[test]
fn binary_proxy_edits_and_disconnect_rolls_back() {
    use std::io::{BufRead, BufReader};
    let f = Fixture::new();
    let id = f.create();
    let mut server = pm_mcp::transport::LoopbackServer::start(&f.root, f.host.clone()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_powermove-mcp"))
        .args(["--powermove-mcp", "--profile"])
        .arg(&f.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut next = 0;
    {
        let mut rpc = |method: &str, params: Value| -> Value {
            next += 1;
            writeln!(
                stdin,
                "{}",
                json!({"jsonrpc":"2.0","id":next,"method":method,"params":params})
            )
            .unwrap();
            stdin.flush().unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let result: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(result["id"], next);
            result["result"].clone()
        };
        rpc("initialize", json!({}));
        assert_eq!(
            rpc("tools/list", json!({}))["tools"],
            json!(catalog::tools())
        );
        value(rpc(
            "tools/call",
            json!({"name":"start_session","arguments":{"prompt":"Synthetic proxy test"}}),
        ));
        value(rpc(
            "tools/call",
            json!({"name":"apply_commands","arguments":{"commands":[{"type":"add_layer","id":"L-proxy","layerType":"solid"}]}}),
        ));
    }
    assert_eq!(f.host.lock().unwrap().store(&id).unwrap().layers().len(), 1);
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.shutdown();
    assert_eq!(f.host.lock().unwrap().store(&id).unwrap().layers().len(), 0);
}
#[test]
fn strip_operations_use_atomic_history_and_keep_input_order() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    value(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":"L-a","layerType":"solid","duration":1},{"type":"add_layer","id":"L-b","layerType":"solid","duration":2}]})));
    value(s.call(
        "edit_layer_strips",
        json!({"operation":"sequence","layerIds":["L-a","L-b"],"at":2,"gap":0.5}),
    ));
    let h = f.host.lock().unwrap();
    let p = h.read_document(&id).unwrap();
    let layers = p["layers"].as_array().unwrap();
    assert_eq!(layers.iter().find(|l| l["id"] == "L-a").unwrap()["from"], 2);
    assert_eq!(
        layers.iter().find(|l| l["id"] == "L-b").unwrap()["from"],
        3.5
    );
    drop(h);
    let before = f.host.lock().unwrap().revision(&id).unwrap();
    diagnostic(
        &mut s,
        "edit_layer_strips",
        json!({"operation":"move","layerIds":["L-a","L-b"],"at":9}),
    );
    assert_eq!(f.host.lock().unwrap().revision(&id).unwrap(), before);
    let copy = value(s.call(
        "edit_layer_strips",
        json!({"operation":"duplicate","layerIds":["L-a"],"delta":1}),
    ));
    assert_eq!(copy["strips"]["copies"][0]["sourceId"], "L-a");
    assert_eq!(f.host.lock().unwrap().store(&id).unwrap().layers().len(), 3);
    value(s.call("finish_session", json!({"commit":false})));
    assert!(
        f.host
            .lock()
            .unwrap()
            .store(&id)
            .unwrap()
            .layers()
            .is_empty()
    );
}
#[test]
fn library_navigation_and_trash_restore_are_durable_boundaries() {
    let f = Fixture::new();
    let first = f.create();
    let mut s = f.session();
    value(s.call(
        "start_session",
        json!({"prompt":"Library test","context":"app"}),
    ));
    let second = value(s.call(
        "manage_project",
        json!({"operation":"create","name":"Second synthetic"}),
    ))["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    value(s.call(
        "manage_project",
        json!({"operation":"trash","projectId":first}),
    ));
    let list = value(s.call("manage_project", json!({"operation":"list"})));
    assert_eq!(list["trash"][0]["id"], first);
    value(s.call(
        "manage_project",
        json!({"operation":"restore","projectId":first}),
    ));
    value(s.call(
        "manage_project",
        json!({"operation":"open","projectId":first}),
    ));
    assert_eq!(
        f.host.lock().unwrap().active.as_deref(),
        Some(first.as_str())
    );
    value(s.call("finish_session", json!({"commit":false})));
    assert!(
        pm_kernel::LibraryStore::open(&f.root)
            .load(&second)
            .unwrap()
            .is_some()
    );
}

#[test]
fn legacy_command_result_bodies_match_tag_source() {
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-a");
    let commands = json!([
        {"type":"set_layer","target":"L-a","patch":{"visible":false,"name":"Renamed fixture"}},
        {"type":"set_composition","patch":{"name":"Renamed composition","width":1280}},
        {"type":"add_marker","id":"marker-fixture","time":1.25,"name":"Cue"},
        {"type":"remove_marker","index":0},
        {"type":"create_section","section":{"id":"section-fixture","layers":[]}},
        {"type":"update_section","sectionId":"section-fixture","layers":[{"id":"Saved-layer","type":"solid"}],"at":1000,"version":{"id":"SV-fixture"}}
    ]);
    let project = f.host.lock().unwrap().read_document(&id).unwrap();
    let expected = node(
        "command-pairs.mjs",
        Some(&json!({"project":project,"commands":commands})),
    );
    let actual = value(s.call("apply_commands", json!({"commands":commands})));
    assert_eq!(actual["results"], expected);
}
#[test]
fn registry_definitions_back_effect_and_transition_commands() {
    use pm_kernel::state::{Registration, StateOp, StateSnapshot, StateTransaction};
    let owner = pm_kernel::Writer {
        realm: 101,
        generation: 1,
    };
    let def = |id: &str| {
        Registration{token:format!("token-{id}"),owner,id:id.into(),definition:pm_model::json::parse(&json!({"id":id,"version":1,"label":"Synthetic definition","params":[{"k":"amount","type":"number","def":0.5}]}).to_string()).unwrap()}
    };
    let (state, _) = StateSnapshot::default()
        .stage(StateTransaction {
            expected_revision: None,
            ops: vec![
                StateOp::Register {
                    registry: "effects".into(),
                    entry: def("synthetic-fx"),
                },
                StateOp::Register {
                    registry: "transitions".into(),
                    entry: def("synthetic-transition"),
                },
            ],
        })
        .unwrap();
    let f = Fixture::new();
    let id = f.create();
    let mut s = f.session();
    start(&mut s);
    add(&mut s, "L-a");
    let p = f.host.lock().unwrap().read_document(&id).unwrap();
    let batch=pm_mcp::commands::plan_with_definitions(&p,&[],&[
        json!({"type":"add_effect","target":"L-a","effect":"synthetic-fx"}),
        json!({"type":"set_transition","layer":"L-a","edge":"in","transition":{"type":"synthetic-transition","dur":0.5}})
    ],0.,Some(&state)).unwrap();
    assert!(batch.results[0]["data"]["effectId"].is_string());
    assert_eq!(
        batch.results[1]["data"]["transition"],
        "synthetic-transition"
    );
    let mut h = f.host.lock().unwrap();
    let revision = h.revision(&id).unwrap();
    h.apply_commands(
        &id,
        batch.commands,
        "Synthetic definitions",
        pm_kernel::Origin::Agent,
        revision,
        904,
    )
    .unwrap();
    assert_eq!(
        h.read_document(&id).unwrap()["layers"][0]["fx"][0]["type"],
        "synthetic-fx"
    );
}

// Internal diagnostics retain codes/backing; MCP wire errors expose the tagged message.
fn diagnostic(session: &mut McpSession, name: &str, args: Value) -> Value {
    serde_json::to_value(
        session
            .dispatch(name, &args)
            .expect_err("expected tool refusal"),
    )
    .unwrap()
}

#[test]
fn transform_layers_runs_over_the_batch_selection() {
    let solid = |id: &str| {
        json!({"id":id,"type":"solid","name":id,"from":0,"dur":5,"on":true,"lock":false,
            "parent":null,"group":null,"fx":[],"masks":[],"d":{"w":10,"h":10,"color":"#FF0000"},
            "p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":0,"kf":[]},"position.y":{"v":0,"kf":[]}}})
    };
    let project = json!({"id":"p","name":"p","compId":"root","w":100,"h":100,"fps":30,"dur":5,
        "assets":{},"comps":{},"layers":[solid("a"),solid("b")]});
    let edit = json!({"type":"transform_layers","transform":{"edits":[
        {"path":"properties.opacity","value":{"op":"multiply","args":[{"ref":"current"},0.5]}}
    ]}});
    let batch =
        pm_mcp::commands::plan(&project, &["b".to_owned()], std::slice::from_ref(&edit), 0.)
            .unwrap();
    assert_eq!(batch.commands.len(), 1);
    let mut store = pm_kernel::DocumentStore::new_untitled(
        pm_model::json::parse(&project.to_string()).unwrap(),
    );
    store
        .transact("Transform", pm_kernel::Origin::Agent, &batch.commands)
        .unwrap();
    let after = store.root().unwrap().to_value();
    let layers = after
        .get("comps")
        .and_then(|c| c.get("root"))
        .unwrap_or(&after)
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap();
    let opacity = |l: &pm_model::Value| {
        let v = l
            .get("p")
            .unwrap()
            .get("opacity")
            .unwrap()
            .get("v")
            .unwrap();
        v.as_f64().unwrap()
    };
    assert_eq!(opacity(&layers[0]), 100., "not selected");
    assert_eq!(opacity(&layers[1]), 50.);
    assert!(pm_mcp::commands::plan(&project, &[], &[edit], 0.).is_err());
}
