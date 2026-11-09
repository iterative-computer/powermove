use super::*;
use pm_model::json::parse;
use serde_json::json;
fn model(value: &Value) -> pm_model::Value {
    parse(&value.to_string()).unwrap()
}
fn json_value(value: &pm_model::Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(value)).unwrap()
}
fn document(p: &Published) -> Value {
    let mut value = json_value(&pm_model::canonical::realize_project(
        p.snapshot.document.to_value().unwrap(),
    ));
    value["revision"] = json!(p.revision());
    value
}
struct Fixture {
    _root: tempfile::TempDir,
    actor: Option<KernelActor>,
    host: Arc<Host>,
    tools: Tools,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        let actor = KernelActor::spawn(KernelConfig {
            journal_root: Some(profile.join("journals")),
            ..Default::default()
        });
        attach(&actor.handle(), &profile).unwrap();
        let (host, _) = Host::new(actor.handle(), profile);
        host.open("test".into(),SessionSource::New(model(&json!({"id":"test","name":"Synthetic","compId":"c","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","layers":[],"assets":{},"comps":{}})))).unwrap();
        host.save(host.current().unwrap(), None).unwrap();
        Self {
            actor: Some(actor),
            tools: Tools::new(host.clone(), host.profile.join("exports")),
            host,
            _root: root,
        }
    }
    fn connection(&self) -> Connection {
        self.tools.connect()
    }
    fn ui(&self, call: impl Into<Call>) -> Ack {
        self.host
            .kernel
            .call_blocking("test", Request::new(Caller::native(), call))
            .unwrap()
    }
    fn add_ui(&self) {
        self.ui(Call::Transact {
            label: Some("UI edit".into()),
            body: WriteBody::Commands(vec![
                CompCommand::Set {
                    patch: Box::new(pm_kernel::commands::comps::CompPatch {
                        name: Some(format!(
                            "UI rename {}",
                            self.host.kernel.published("test").unwrap().revision()
                        )),
                        ..Default::default()
                    }),
                }
                .into(),
            ]),
            expected_revision: None,
        });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.actor.take().unwrap().shutdown();
    }
}
fn start(c: &mut Connection) {
    c.dispatch("start_session", &json!({"prompt":"Synthetic test"}))
        .unwrap();
}
fn add(c: &mut Connection, id: &str) {
    c.dispatch(
        "apply_commands",
        &json!({"commands":[{"type":"add_layer","id":id,"layerType":"solid","name":"Synthetic"}]}),
    )
    .unwrap();
}
fn layers(f: &Fixture) -> usize {
    document(&f.host.kernel.published("test").unwrap())["layers"]
        .as_array()
        .unwrap()
        .len()
}
/// What the agent panel saved is what `thread_read` returns.
#[test]
fn thread_tools_page_the_durable_archive() {
    let fixture = Fixture::new();
    crate::mcp_services::connect_engine(&fixture.tools, Default::default()).unwrap();
    let long = "é".repeat(30);
    let archive = json!({"version":1,"threads":[{"id":"t1","title":"Headline",
        "updatedAt":5,"provider":"claude","access":"project","lastRunStatus":"completed",
        "lastRunId":"r1","conversation":[{"role":"user","text":long},
        {"role":"assistant","text":"Done."}]}]});
    fixture
        .host
        .kernel
        .state_transaction(state::StateTransaction {
            expected_revision: None,
            ops: vec![state::StateOp::Setting {
                key: "agentThreads.test".into(),
                value: Some(model(&archive)),
            }],
        })
        .wait()
        .unwrap()
        .unwrap();
    let mut c = fixture.connection();
    c.dispatch("start_session", &json!({"prompt":"read"}))
        .unwrap();
    let listed = c.dispatch("thread_list", &json!({})).unwrap();
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["threads"][0]["threadId"], "t1");
    assert_eq!(listed["threads"][0]["status"], "completed");
    assert_eq!(listed["threads"][0]["busy"], false);
    assert_eq!(listed["threads"][0]["summary"], "Done.");
    let read = c
        .dispatch(
            "thread_read",
            &json!({"threadId":"t1","limit":1,"maxCharsPerMessage":10}),
        )
        .unwrap();
    assert_eq!(read["messages"][0]["text"], "é".repeat(10));
    assert_eq!(read["messages"][0]["nextTextOffset"], 10);
    assert_eq!(read["nextOffset"], 1);
    assert!(
        c.dispatch("thread_read", &json!({"threadId":"t2"}))
            .is_err()
    );
}
#[test]
fn live_mcp_folds_into_ui_gesture_without_regrouping_or_rollback() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    let Reply::Gesture(gesture) = f
        .ui(GestureCall::Begin {
            label: "UI drag".into(),
        })
        .reply
    else {
        panic!()
    };
    add(&mut c, "agent");
    assert!(f.host.kernel.published("test").unwrap().entry.is_some());
    assert_eq!(
        c.dispatch("finish_session", &json!({"commit":false}))
            .unwrap_err()
            .code,
        "revision_conflict"
    );
    assert_eq!(layers(&f), 1);
    f.ui(GestureCall::Commit { gesture });
    let p = f.host.kernel.published("test").unwrap();
    assert_eq!(p.history.entries, 1);
    assert_eq!(p.history.undo_label.as_deref(), Some("UI drag"));
}
#[test]
fn uninterrupted_run_groups_and_disconnect_rolls_back() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    add(&mut c, "one");
    add(&mut c, "two");
    c.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    assert_eq!(f.host.kernel.published("test").unwrap().history.entries, 1);
    start(&mut c);
    add(&mut c, "three");
    c.disconnect().unwrap();
    assert_eq!(layers(&f), 2);
}
#[test]
fn stale_and_interleaved_runs_preserve_foreign_work() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    f.add_ui();
    assert_eq!(
        c.dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","id":"stale","layerType":"solid"}]})
        )
        .unwrap_err()
        .code,
        "revision_conflict"
    );
    c.dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
    add(&mut c, "agent");
    f.add_ui();
    c.dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
    assert_eq!(
        c.dispatch("finish_session", &json!({"commit":false}))
            .unwrap_err()
            .code,
        "revision_conflict"
    );
    assert_eq!(layers(&f), 1);
    assert!(
        document(&f.host.kernel.published("test").unwrap())["name"]
            .as_str()
            .unwrap()
            .starts_with("UI rename")
    );
}
#[test]
fn no_op_then_foreign_edit_refreshes_rollback_baseline() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    c.dispatch(
        "apply_commands",
        &json!({"commands":[{"type":"set_composition","patch":{"name":"Synthetic"}}]}),
    )
    .unwrap();
    f.add_ui();
    c.dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
    add(&mut c, "agent");
    c.dispatch("finish_session", &json!({"commit":false}))
        .unwrap();
    assert!(
        document(&f.host.kernel.published("test").unwrap())["name"]
            .as_str()
            .unwrap()
            .starts_with("UI rename")
    );
    assert_eq!(layers(&f), 0);
}
#[test]
fn reopened_identity_and_editor_permissions_are_fenced() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    f.host
        .shell(ShellCall::Close(f.host.current().unwrap()))
        .unwrap();
    f.host
        .open(
            "test".into(),
            SessionSource::Library {
                profile: f.host.profile.clone(),
                project: "test".into(),
            },
        )
        .unwrap();
    // A replaced document retires the run, as on main; it never reaches the new one.
    assert_eq!(
        c.dispatch("get_project_state", &json!({"indexOnly":true}))
            .unwrap_err()
            .message,
        "Call start_session before using Powermove tools."
    );
    let mut c = f.connection();
    c.dispatch(
        "start_session",
        &json!({"prompt":"Inspection","access":"editor"}),
    )
    .unwrap();
    c.dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
    assert!(
        c.dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_layer","layerType":"solid"}]})
        )
        .is_err()
    );
}
#[test]
fn closing_gate_refuses_new_calls_and_export_preserves_the_live_document() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    add(&mut c, "saved");
    let save = c.dispatch("save_project", &json!({})).unwrap();
    assert!(std::path::Path::new(save["path"].as_str().unwrap()).exists());
    assert!(f.host.kernel.published("test").unwrap().dirty);
    assert!(f.host.kernel.published("test").unwrap().path.is_none());
    let operation = f.host.try_operation().unwrap();
    assert_eq!(
        c.dispatch("get_project_state", &json!({"indexOnly":true}))
            .unwrap_err()
            .code,
        "revision_conflict"
    );
    drop(operation);
    c.dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
    f.host.closing.store(true, Ordering::Release);
    assert_eq!(
        c.dispatch("get_project_state", &json!({"indexOnly":true}))
            .unwrap_err()
            .code,
        "revision_conflict"
    );
}

#[test]
fn library_tools_and_app_context_use_the_same_native_session() {
    let f = Fixture::new();
    let mut c = f.connection();
    assert!(
        !c.dispatch("list_projects", &json!({})).unwrap()["projects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let created = c
        .dispatch(
            "create_project",
            &json!({"name":"Created","width":320,"height":180,"duration":2}),
        )
        .unwrap();
    let id = created["projectId"].as_str().unwrap();
    c.dispatch("open_project", &json!({"projectId":id}))
        .unwrap();
    c.dispatch(
        "start_session",
        &json!({"prompt":"App library test","context":"app"}),
    )
    .unwrap();
    c.dispatch("manage_project", &json!({"operation":"list"}))
        .unwrap();
    let duplicate = c
        .dispatch(
            "manage_project",
            &json!({"operation":"duplicate","projectId":id,"name":"Copy"}),
        )
        .unwrap();
    let copy = duplicate["projectId"].as_str().unwrap();
    c.dispatch(
        "manage_project",
        &json!({"operation":"trash","projectId":copy}),
    )
    .unwrap();
    c.dispatch(
        "manage_project",
        &json!({"operation":"restore","projectId":copy}),
    )
    .unwrap();
    assert_eq!(
        c.dispatch("finish_session", &json!({"commit":true}))
            .unwrap()["changed"],
        true
    );
    start(&mut c);
    assert_eq!(
        c.dispatch("get_project_state", &json!({"indexOnly":true}))
            .unwrap()["composition"]["workArea"],
        json!([0, 2])
    );
    c.dispatch(
        "manage_project",
        &json!({"operation":"rename","name":"Renamed"}),
    )
    .unwrap();
    c.dispatch("manage_project", &json!({"operation":"save"}))
        .unwrap();
    c.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
}
#[test]
fn selection_timeline_strips_and_explicit_rollback_share_publications() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    add(&mut c, "strip");
    c.dispatch("select_layers", &json!({"layerIds":["strip"]}))
        .unwrap();
    assert_eq!(
        f.host.kernel.published("test").unwrap().selection.primary(),
        Some("strip")
    );
    let timeline = c.dispatch("get_timeline", &json!({})).unwrap();
    assert_eq!(timeline["total"], 1);
    let changed = c
        .dispatch(
            "edit_layer_strips",
            &json!({"operation":"state","layerIds":["strip"],"patch":{"name":"Changed"}}),
        )
        .unwrap();
    assert_eq!(
        changed["strips"]["revision"],
        f.host.kernel.published("test").unwrap().revision()
    );
    c.dispatch("rollback_changes", &json!({})).unwrap();
    assert_eq!(layers(&f), 0);
    c.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
}
#[test]
fn implemented_edits_keep_mcp_origin_in_durable_history() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    add(&mut c, "origin");
    c.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    f.host.kernel.sync().wait().unwrap();
    let recovery = f.host.kernel.recover("test", None, None).wait().unwrap();
    let pm_kernel::journal::Recovery::Recovered(recovered) = recovery else {
        panic!("Expected a recoverable journal")
    };
    assert_eq!(
        recovered.history.records().last().unwrap().origin,
        Origin::Mcp
    );
}

#[test]
fn live_app_mcp_exports_unique_packed_library_files_without_adopting_a_destination() {
    let f = Fixture::new();
    let mut connection = f.connection();
    start(&mut connection);
    add(&mut connection, "changed");
    let before = f.host.kernel.published("test").unwrap();
    let mut paths = Vec::new();
    for _ in 0..2 {
        let response = connection.dispatch("save_project", &json!({})).unwrap();
        let path = PathBuf::from(response["path"].as_str().unwrap());
        assert_eq!(
            path.parent(),
            Some(f.host.profile.join("exports").as_path())
        );
        let opened = pm_kernel::open_project(&path, Default::default()).unwrap();
        assert_eq!(
            opened.document.to_value(),
            before.snapshot.document.to_value()
        );
        paths.push(path);
        let after = f.host.kernel.published("test").unwrap();
        assert_eq!(after.path, before.path);
        assert_eq!(after.revision(), before.revision());
        assert_eq!(after.dirty, before.dirty);
        assert_eq!(after.history.entries, before.history.entries);
        assert_eq!(after.saving, before.saving);
    }
    assert_ne!(paths[0], paths[1]);
    assert!(f.host.kernel.published("test").unwrap().path.is_none());
    connection
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
}

#[test]
fn live_app_ingress_and_actor_export_keep_authored_utf16_and_source_bytes() {
    let f = Fixture::new();
    let source = f._root.path().join("utf16.pmv");
    let bytes =
        include_bytes!("../../../../../engine/crates/pm-model/tests/fixtures/containers/pmv4.pmv");
    std::fs::write(&source, bytes).unwrap();
    let opened = f
        .host
        .open("utf16".into(), SessionSource::File(source.clone()))
        .unwrap();
    assert_eq!(opened["projectId"], "utf16");
    let current = f.host.current().unwrap();
    let before = f.host.kernel.published(&current.project).unwrap();
    let saved = f._root.path().join("actor-export.pmv");
    f.host
        .kernel
        .prepare_export(current, before.snapshot.doc_id, saved.clone())
        .wait()
        .unwrap()
        .unwrap()
        .run()
        .unwrap();
    let opened = pm_kernel::open_project(&saved, Default::default()).unwrap();
    assert_eq!(
        opened.document.to_value(),
        before.snapshot.document.to_value()
    );
    assert!(
        pm_model::json::to_canonical_string(&opened.document.to_value().unwrap())
            .contains("\\ud800")
    );
    assert_eq!(std::fs::read(source).unwrap(), bytes);
}

#[test]
fn queued_app_mcp_call_cannot_edit_a_reopened_document_with_the_same_project_id() {
    let f = Fixture::new();
    let retired = f.host.kernel.published("test").unwrap().snapshot.doc_id;
    assert!(f.host.kernel.close("test").wait().unwrap());
    f.host.kernel.open("test",OpenSource::Untitled(model(&json!({"id":"test","name":"Replacement","compId":"c","w":320,"h":180,"fps":30,"dur":2,"layers":[],"assets":{},"comps":{}})))).wait().unwrap().unwrap();
    let before = f.host.kernel.published("test").unwrap();
    let mut automation = f.tools.automation.lock().unwrap();
    automation.document_fence = Some(retired);
    let retired_request = automation.apply_commands(
        "test",
        vec![],
        "Retired request",
        Origin::Mcp,
        before.revision(),
        7,
    );
    assert!(retired_request.is_err());
    drop(automation);
    let after = f.host.kernel.published("test").unwrap();
    assert_eq!(after.revision(), before.revision());
    assert!(after.entry.is_none());
    assert_eq!(
        after.snapshot.document.to_value(),
        before.snapshot.document.to_value()
    );
}

#[test]
fn live_app_named_mcp_save_is_in_place_and_stale_document_saves_are_refused() {
    let f = Fixture::new();
    let source = f._root.path().join("working.pmv");
    f.host
        .save(f.host.current().unwrap(), Some(source.clone()))
        .unwrap();
    let original = std::fs::read(&source).unwrap();
    let mut connection = f.connection();
    start(&mut connection);
    add(&mut connection, "named-edit");
    let before = f.host.kernel.published("test").unwrap();
    assert!(before.dirty);
    for _ in 0..2 {
        let response = connection.dispatch("save_project", &json!({})).unwrap();
        assert_eq!(response["path"], json!(source));
        let after = f.host.kernel.published("test").unwrap();
        assert_eq!(after.path.as_ref(), Some(&source));
        assert_eq!(after.revision(), before.revision());
        assert!(!after.dirty);
        let opened = pm_kernel::open_project(&source, Default::default()).unwrap();
        assert_eq!(
            opened.document.to_value(),
            before.snapshot.document.to_value()
        );
    }
    assert_ne!(std::fs::read(&source).unwrap(), original);
    assert!(!f.host.profile.join("exports").exists());
    let bytes = std::fs::read(&source).unwrap();
    let refused = f
        .host
        .kernel
        .prepare_document_save(f.host.current().unwrap(), before.snapshot.doc_id + 1)
        .wait()
        .unwrap();
    assert!(refused.is_err());
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    assert!(!f.host.kernel.published("test").unwrap().saving);
    connection
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
}

#[test]
fn app_memory_uses_durable_metadata_and_survives_agent_rollback_and_reopen() {
    let f = Fixture::new();
    let before = f.host.kernel.published("test").unwrap();
    let mut c = f.connection();
    start(&mut c);
    let added = c.dispatch("project_memory", &json!({"operation":"add", "kind":"decision", "text":"Synthetic intent", "why":"Synthetic reason"})).unwrap();
    let noted = f.host.kernel.published("test").unwrap();
    assert_eq!(noted.revision(), before.revision());
    assert_eq!(noted.history.entries, before.history.entries);
    assert_eq!(noted.dirty, before.dirty);
    assert!(noted.metadata_revision > before.metadata_revision);
    assert_eq!(added["entry"]["author"], "agent");
    add(&mut c, "temporary");
    c.dispatch("project_memory", &json!({"operation":"update", "id":added["entry"]["id"], "text":"Retained intent", "resolved":true})).unwrap();
    c.dispatch("rollback_changes", &json!({})).unwrap();
    let listed = c
        .dispatch("project_memory", &json!({"operation":"list"}))
        .unwrap();
    assert_eq!(listed["memory"][0]["text"], "Retained intent");
    assert_eq!(layers(&f), 0);
    c.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    let session = f.host.current().unwrap();
    f.host.shell(ShellCall::Close(session)).unwrap();
    f.host.bind(None);
    f.host
        .open(
            "test".into(),
            SessionSource::Library {
                profile: f.host.profile.clone(),
                project: "test".into(),
            },
        )
        .unwrap();
    let mut reopened = f.connection();
    start(&mut reopened);
    assert_eq!(
        reopened
            .dispatch("project_memory", &json!({"operation":"list"}))
            .unwrap()["memory"],
        listed["memory"]
    );
}

#[test]
fn app_agent_inputs_hide_disabled_active_and_retired_notes_without_deleting_them() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    let first = c
        .dispatch(
            "project_memory",
            &json!({"operation":"add","kind":"decision","text":"Synthetic retired intent"}),
        )
        .unwrap();
    c.dispatch(
        "project_memory",
        &json!({"operation":"remove","id":first["entry"]["id"]}),
    )
    .unwrap();
    c.dispatch(
        "project_memory",
        &json!({"operation":"add","kind":"goal","text":"Synthetic active intent"}),
    )
    .unwrap();
    c.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    let before = f.host.kernel.published("test").unwrap();
    let ack = f
        .host
        .kernel
        .call_blocking(
            "test",
            Request::new(
                Caller::native(),
                pm_kernel::MemoryCall::SetEnabled {
                    expected: None,
                    enabled: false,
                },
            )
            .with_document(before.snapshot.doc_id),
        )
        .unwrap();
    assert!(matches!(ack.reply, Reply::Memory { .. }), "{ack:?}");
    let mut next = f.connection();
    let started = next
        .dispatch(
            "start_session",
            &json!({"prompt":"Synthetic disabled notes"}),
        )
        .unwrap();
    let input: Value = serde_json::from_slice(
        &std::fs::read(
            PathBuf::from(started["workspace"].as_str().unwrap())
                .join("inputs/powermove-project.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(input.get("memory").is_none());
    assert!(input.get("nativeMemoryArchive").is_none());
    assert_eq!(
        next.dispatch("project_memory", &json!({"operation":"list"}))
            .unwrap_err()
            .message,
        pm_mcp::memory::DISABLED
    );
    assert_eq!(
        next.dispatch("get_project_state", &json!({"indexOnly":true}))
            .unwrap()["memory"],
        json!({"enabled":false})
    );
    let after = f.host.kernel.published("test").unwrap();
    assert_eq!(
        after.snapshot.root().unwrap().get("memory"),
        before.snapshot.root().unwrap().get("memory")
    );
    assert_eq!(
        after.snapshot.root().unwrap().get("nativeMemoryArchive"),
        before.snapshot.root().unwrap().get("nativeMemoryArchive")
    );
    assert_eq!(after.revision(), before.revision());
}

#[test]
fn app_memory_wire_keeps_split_utf16_emoji_through_reads_and_updates() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    let decode = |wire: Value| {
        let wire: Value = serde_json::from_str(&wire.to_string()).unwrap();
        assert_eq!(wire["isError"], false, "{wire}");
        pm_model::json::parse(wire["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let published = f.host.kernel.published("test").unwrap();
    let ack = f
        .host
        .kernel
        .call_blocking(
            "test",
            Request::new(
                Caller::native(),
                pm_kernel::MemoryCall::Mutate {
                    expected: None,
                    operation: pm_model::memory::Operation::Add {
                        kind: "decision".into(),
                        text: format!("{}😀", "x".repeat(599)).as_str().into(),
                        why: None,
                        resolved: false,
                    },
                    now: 1000.,
                    id: "utf16-note".into(),
                },
            )
            .with_document(published.snapshot.doc_id),
        )
        .unwrap();
    assert!(matches!(ack.reply, Reply::Memory { .. }), "{ack:?}");
    let added = decode(
        c.invoke("project_memory", json!({"operation":"list"}))
            .unwrap(),
    );
    let entry = &added.get("memory").unwrap().as_array().unwrap()[0];
    let text = entry.get("text").unwrap().as_js_string().unwrap();
    assert_eq!(text.utf16_units().count(), 600);
    assert_eq!(text.utf16_units().last(), Some(0xd83d));
    let id = entry.get("id").unwrap().as_str().unwrap();
    let updated = decode(
        c.invoke(
            "project_memory",
            json!({"operation":"update","id":id,"resolved":true}),
        )
        .unwrap(),
    );
    assert_eq!(updated.get("entry").unwrap().get("text"), entry.get("text"));
    let listed = decode(
        c.invoke("project_memory", json!({"operation":"list"}))
            .unwrap(),
    );
    assert_eq!(
        listed.get("memory").unwrap().as_array().unwrap()[0],
        *updated.get("entry").unwrap()
    );
    assert_eq!(
        f.host.kernel.published("test").unwrap().revision(),
        published.revision()
    );
}
#[test]
fn a_recovery_choice_in_the_window_is_named_and_library_reads_still_run() {
    let f = Fixture::new();
    let mut c = f.connection();
    start(&mut c);
    let operation = f.host.try_operation().unwrap();
    f.host.awaiting_recovery(Some("Synthetic".into()));
    let refused = c
        .dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap_err();
    assert_eq!(refused.code, "revision_conflict");
    assert!(
        refused.message.contains("“Synthetic” has unsaved work"),
        "{}",
        refused.message
    );
    assert!(refused.message.contains("waiting for someone to choose"));
    c.dispatch("list_projects", &json!({})).unwrap();
    f.host.awaiting_recovery(None);
    drop(operation);
    c.dispatch("get_project_state", &json!({"indexOnly":true}))
        .unwrap();
}
