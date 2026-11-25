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
        let host = Host::new(actor.handle(), profile);
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
    crate::mcp_services::connect_engine(&fixture.tools, Default::default(), Default::default())
        .unwrap();
    let long = "é".repeat(30);
    let archive = json!({"version":1,"threads":[{"id":"t1","title":"Headline",
        "updatedAt":5,"provider":"claude","access":"project","lastRunStatus":"completed",
        "lastRunId":"r1","conversation":[{"role":"user","text":long},
        {"role":"assistant","text":"Done."}]}]});
    use pm_agents::persistence::Store;
    let mut store = pm_panel_agentpanel::store::KernelStore::bind(fixture.host.kernel.clone())
        .unwrap();
    assert!(store.get("agentThreads.test").unwrap().is_none());
    store.set("agentThreads.test", &archive).unwrap();
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
    let mut automation = pm_mcp::Automation::fenced(&f.tools.automation, Some(retired)).unwrap();
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
    f.host.bind_window(0, None);
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

#[::core::prelude::v1::test]
fn only_agent_stills_draw_offline_cards() {
    use crate::mcp_services::Capture;
    assert!(Capture::AgentStill.draws_offline_cards());
    assert!(!Capture::ExtensionSnapshot.draws_offline_cards());
    assert!(!Capture::Sheet.draws_offline_cards());
    assert!(
        !Capture::Poster.draws_offline_cards(),
        "a poster draws offline media as nothing"
    );
}

/// Each editor window owns its project. MCP acts in the key window, a project a
/// window already shows stays there, and MCP never closes a window's project
/// underneath it (the window does, once it shows the new one).
#[test]
fn window_jumps_import_completion_keeps_the_key_windows_project() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(profile.join("journals")),
        ..Default::default()
    });
    attach(&actor.handle(), &profile).unwrap();
    let host = Host::new(actor.handle(), profile);
    let first = host.attach_window(1);
    let second = host.attach_window(2);
    let woke = |rx: &async_channel::Receiver<()>| rx.try_recv().is_ok();
    let project = |id: &str| {
        SessionSource::New(model(
            &json!({"id":id,"name":id,"compId":"c","w":320,"h":180,"fps":30,"dur":2,"bg":"#000000","layers":[],"assets":{},"comps":{}}),
        ))
    };
    // The first window attached is key: MCP's project lands there, and only
    // that window hears about it.
    host.open("alpha".into(), project("alpha")).unwrap();
    // The window takes it (follow_mcp) and shows it.
    let alpha = host.claim_placed(1).expect("alpha in window 1");
    assert!(woke(&first));
    assert!(!woke(&second), "window 2 never follows window 1");
    assert_eq!(host.window_session(2), None);
    // Window 2 opens its own project from its UI: window 1 keeps alpha.
    host.activate_window(2);
    let beta = match host
        .shell(ShellCall::PrepareOpen {
            id: "beta".into(),
            source: project("beta"),
            electron_profile: None,
        })
        .unwrap()
    {
        ShellReply::Opened { session, .. } => session,
        other => panic!("{other:?}"),
    };
    host.bind_window(2, Some(beta.clone()));
    assert!(
        !woke(&first) && !woke(&second),
        "a window's own open notifies nobody"
    );
    assert_eq!(host.window_session(1), Some(alpha.clone()));
    assert_eq!(
        host.current(),
        Some(beta.clone()),
        "MCP means the key window's"
    );
    // A real import finishes in window 1 after window 2 took focus.
    let still = host.profile.join("focus-test.png");
    std::fs::write(&still, crate::mcp_services::png(&[20, 40, 60, 255], 1, 1).unwrap()).unwrap();
    let panels = pm_builtins::Builtins::new(actor.handle(), &alpha.project, || {}).unwrap();
    let outcome = panels
        .import_media(
            actor.handle().published(&alpha.project).unwrap().snapshot.doc_id,
            &host.profile,
            pm_builtins::media_import::ImportRequest {
                paths: vec![still],
                ..Default::default()
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.imported, ["focus-test.png"]);
    // The window reports its own session when applying the publication.
    host.bind_window(1, Some(alpha.clone()));
    assert_eq!(host.current(), Some(beta.clone()));
    assert!(!woke(&first) && !woke(&second));
    // Tools find a project in whichever window shows it.
    assert_eq!(host.session_of("alpha"), Some(alpha.clone()));
    // open_project of a project window 1 shows: window 1 becomes MCP's, nothing
    // is replaced anywhere.
    host.open(
        "alpha".into(),
        SessionSource::Library {
            profile: host.profile.clone(),
            project: "alpha".into(),
        },
    )
    .unwrap();
    assert_eq!(host.window_session(2), Some(beta.clone()));
    assert_eq!(host.current(), Some(alpha.clone()));
    assert!(!woke(&first) && !woke(&second));
    // An empty window is preferred over replacing a project.
    let third = host.attach_window(3);
    host.open("delta".into(), project("delta")).unwrap();
    assert!(woke(&third) && !woke(&first) && !woke(&second));
    assert_eq!(host.claim_placed(3).unwrap().project, "delta");
    assert_eq!(
        host.current().unwrap().project,
        "delta",
        "and becomes MCP's"
    );
    // Every window has a project: the key window's is replaced (v1's one
    // window). MCP places it, the window is told, and alpha stays open until
    // the window closes it itself.
    host.activate_window(1);
    host.open("gamma".into(), project("gamma")).unwrap();
    assert!(woke(&first) && !woke(&second) && !woke(&third));
    assert_eq!(host.window_session(1).unwrap().project, "gamma");
    assert!(
        host.kernel.published("alpha").unwrap().session.is_some(),
        "MCP doesn't close a project a window still shows"
    );
    assert_eq!(host.window_session(2), Some(beta));
    // A closed window's project is no longer anyone's.
    host.detach_window(1);
    assert_eq!(host.session_of("gamma"), None);
    // Two opens before a window catches up: the first placement nobody took is
    // closed (no session or lock left behind); the window takes the second.
    let fourth = host.attach_window(4);
    host.activate_window(4);
    host.open("epsilon".into(), project("epsilon")).unwrap();
    host.open("zeta".into(), project("zeta")).unwrap();
    assert!(woke(&fourth));
    assert!(
        host.kernel
            .published("epsilon")
            .is_none_or(|p| p.session.is_none()),
        "the displaced placement was closed"
    );
    assert_eq!(host.claim_placed(4).unwrap().project, "zeta");
    // Once taken, a later open leaves it to the window to close.
    host.open("eta".into(), project("eta")).unwrap();
    assert!(host.kernel.published("zeta").unwrap().session.is_some());
    assert_eq!(host.window_session(4).unwrap().project, "eta");
    actor.shutdown();
}
/// A provider run's tool server carries its grant token on connect; the app's
/// own connection hands it to the session, so orchestration acts as that run
/// while it lives, and a revoked or forged token acts as no run.
#[test]
fn a_connection_acts_as_the_run_that_launched_it() {
    struct Recorder(std::sync::Mutex<Vec<Value>>);
    impl pm_mcp::automation::SurfaceHook for Recorder {
        fn call(&self, _: &str, _: Option<&str>, args: &Value) -> pm_mcp::Result<Value> {
            self.0.lock().unwrap().push(args.clone());
            Ok(json!({}))
        }
    }
    let f = Fixture::new();
    let recorder = Arc::new(Recorder(Default::default()));
    {
        let mut automation = f.tools.automation.lock().unwrap();
        automation.surfaces = Some(recorder.clone());
        automation.agent_grants = Some(Arc::new(|token: &str| {
            pm_agents::launch::grants::resolve(token).map(|(run, _)| (run, "project".to_owned()))
        }));
    }
    let mut c = f.connection();
    start(&mut c);
    let refused = c
        .dispatch("orchestrator_capabilities", &json!({}))
        .unwrap_err();
    assert!(
        refused.message.contains("Powermove agent run"),
        "{}",
        refused.message
    );
    let grant =
        pm_agents::launch::grants::issue("run-9", pm_agents::Access::Project, None).unwrap();
    let mut c = f.connection();
    pm_mcp::transport::ToolConnection::bind_agent_run(&mut c, grant.token());
    start(&mut c);
    c.dispatch("orchestrator_capabilities", &json!({})).unwrap();
    assert_eq!(recorder.0.lock().unwrap()[0]["agentRunId"], "run-9");
    // The run ended: its token no longer acts as it.
    drop(grant);
    assert!(c.dispatch("orchestrator_capabilities", &json!({})).is_err());
    let mut forged = f.connection();
    pm_mcp::transport::ToolConnection::bind_agent_run(&mut forged, "run-9");
    assert!(
        forged
            .dispatch("start_session", &json!({"prompt":"x"}))
            .is_err()
    );
    assert_eq!(recorder.0.lock().unwrap().len(), 1);
}

/// No page, viewer or window realm: jobs still use the production extension
/// discovery, saved enable state and isolated Store registration path.
#[test]
fn unwindowed_jobs_pin_enabled_local_and_store_effects() {
    let f = Fixture::new();
    for (id, store, enabled) in [
        ("synthetic-local", false, true),
        ("synthetic-store", true, true),
        ("synthetic-disabled", false, false),
    ] {
        let folder = f.host.profile.join("extensions").join(id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("manifest.json"),
            json!({
                "id":id,"name":id,"version":"1.0.0","apiVersion":3
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(folder.join("index.ts"), format!(
            "export default function(api){{api.effects.register({{id:'{id}.tint',label:'Tint',group:'Stylize',params:[],frag:'o = vec4(1.0,0.0,0.0,1.0);'}})}}"
        )).unwrap();
        if store {
            std::fs::write(
                folder.join(".powermove-store.json"),
                r#"{"repoId":"synthetic","releaseId":"synthetic"}"#,
            )
            .unwrap();
        }
        if !enabled {
            f.host
                .kernel
                .state_transaction(state::StateTransaction {
                    expected_revision: None,
                    ops: vec![state::StateOp::Extension(state::ExtensionEntry {
                        id: id.into(),
                        installed: true,
                        enabled,
                        version: "1.0.0".into(),
                        source: model(&json!({})),
                    })],
                })
                .wait()
                .unwrap()
                .unwrap();
        }
    }
    let jobs =
        crate::mcp_services::connect_engine(&f.tools, Default::default(), Default::default())
            .unwrap();
    let published = f.host.kernel.published("test").unwrap();
    let first = jobs.resources(&published, true, &|| false).unwrap();
    let ids: std::collections::BTreeSet<_> =
        first.definitions.iter().map(|d| d.id.as_str()).collect();
    assert!(ids.contains("synthetic-local.tint"), "{ids:?}");
    assert!(ids.contains("synthetic-store.tint"), "{ids:?}");
    assert!(!ids.contains("synthetic-disabled.tint"));
    assert_eq!(
        first
            .definitions
            .iter()
            .find(|d| d.id.as_str() == "synthetic-store.tint")
            .unwrap()
            .origin,
        pm_protocol::definitions::DefinitionOrigin::Store
    );
    first.validate().unwrap();
    let entry = f.host.kernel.state().extensions["synthetic-local"].clone();
    f.host
        .kernel
        .state_transaction(state::StateTransaction {
            expected_revision: None,
            ops: vec![state::StateOp::Extension(state::ExtensionEntry {
                enabled: false,
                ..entry.clone()
            })],
        })
        .wait()
        .unwrap()
        .unwrap();
    let disabled = jobs.resources(&published, true, &|| false).unwrap();
    assert!(
        !disabled
            .definitions
            .iter()
            .any(|d| d.id.as_str() == "synthetic-local.tint")
    );
    assert!(
        first
            .definitions
            .iter()
            .any(|d| d.id.as_str() == "synthetic-local.tint")
    );
    f.host
        .kernel
        .state_transaction(state::StateTransaction {
            expected_revision: None,
            ops: vec![state::StateOp::Extension(entry)],
        })
        .wait()
        .unwrap()
        .unwrap();
    // Pins are immutable and the next request reuses the running host.
    assert_eq!(
        first.definitions,
        jobs.resources(&published, true, &|| false)
            .unwrap()
            .definitions
    );
    let window = Arc::new(pm_engine_service::resources::ResourceSnapshot {
        generation: 99,
        definitions: first.definitions.clone(),
        unavailable: first.unavailable.clone(),
        ..Default::default()
    });
    crate::mcp_services::set_resources(published.snapshot.doc_id, Some(window.clone()));
    assert!(Arc::ptr_eq(
        &window,
        &jobs.resources(&published, true, &|| false).unwrap()
    ));
    crate::mcp_services::set_resources(published.snapshot.doc_id, None);
    assert_eq!(
        first.definitions,
        jobs.resources(&published, true, &|| false)
            .unwrap()
            .definitions
    );
}

/// A job of a project no window has open stops waiting for its extensions
/// when it is cancelled (Background Tasks, a stopped capture), at once.
#[test]
fn a_cancelled_job_without_a_window_stops_waiting_for_its_extensions() {
    let f = Fixture::new();
    let folder = f.host.profile.join("extensions/synthetic-slow");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("manifest.json"),
        json!({"id":"synthetic-slow","name":"Slow","version":"1.0.0","apiVersion":3}).to_string(),
    )
    .unwrap();
    std::fs::write(
        folder.join("index.ts"),
        "export default async function(){await new Promise(r=>setTimeout(r,8000))}",
    )
    .unwrap();
    let jobs =
        crate::mcp_services::connect_engine(&f.tools, Default::default(), Default::default())
            .unwrap();
    let published = f.host.kernel.published("test").unwrap();
    let started = std::time::Instant::now();
    let cancelled = || started.elapsed() > std::time::Duration::from_millis(400);
    let answer = jobs.resources(&published, true, &cancelled);
    assert!(
        answer.is_err(),
        "a cancelled wait answers with no definitions"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "the cancel was honoured after {:?}",
        started.elapsed()
    );
}
/// The window's commands are served beside the pinned catalog (which keeps
/// its identity): list_commands and run_command reach the attached project's
/// window, and run_command counts as one of the run's edits.
#[test]
fn run_command_and_list_commands_reach_the_projects_window() {
    type Calls = std::sync::Mutex<Vec<(String, Option<String>, Value)>>;
    struct Recorder(Calls);
    impl pm_mcp::automation::SurfaceHook for Recorder {
        fn call(&self, tool: &str, project: Option<&str>, args: &Value) -> pm_mcp::Result<Value> {
            self.0
                .lock()
                .unwrap()
                .push((tool.into(), project.map(str::to_owned), args.clone()));
            Ok(json!({"ran": true}))
        }
    }
    let f = Fixture::new();
    let recorder = Arc::new(Recorder(Default::default()));
    f.tools.automation.lock().unwrap().surfaces = Some(recorder.clone());
    let mut c = f.connection();
    let listed: Vec<String> = pm_mcp::transport::ToolConnection::list(&mut c)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect();
    assert!(listed.contains(&"run_command".into()) && listed.contains(&"list_commands".into()));
    assert!(
        pm_mcp::catalog::tools()
            .iter()
            .all(|t| t["name"] != "run_command"),
        "the pinned catalog is unchanged"
    );
    assert!(pm_mcp::catalog::names().contains(&"run_command".into()));
    start(&mut c);
    c.dispatch("list_commands", &json!({"query": "ripple"}))
        .unwrap();
    c.dispatch(
        "run_command",
        &json!({"command": "slipLayers", "args": [-1]}),
    )
    .unwrap();
    assert!(
        c.dispatch("run_command", &json!({})).is_err(),
        "a command is required"
    );
    assert!(
        c.dispatch("run_command", &json!({"command": "play", "extra": 1}))
            .is_err()
    );
    let calls = recorder.0.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "list_commands");
    assert_eq!(calls[1].0, "run_command");
    assert_eq!(calls[1].1.as_deref(), Some("test"), "the session's project");
    assert_eq!(calls[1].2["args"], json!([-1]));
}
/// Acceptance runs 24 and 27 (PM-454): a new project made from a tab while
/// another tab's open held the fence (waiting on its recovery question) was
/// dropped and its tab stayed Home. The adopting window now waits for the
/// fence and takes it the moment the other open lets go.
#[gpui::test]
async fn a_new_tabs_project_waits_for_the_fence_instead_of_being_dropped(
    cx: &mut gpui::TestAppContext,
) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(profile.join("journals")),
        ..Default::default()
    });
    attach(&actor.handle(), &profile).unwrap();
    let host = Host::new(actor.handle(), profile);
    let recovering = host.try_operation().expect("the other tab's open holds it");
    let executor = cx.executor();
    let waiting = {
        let host = host.clone();
        let executor = executor.clone();
        cx.background_executor
            .spawn(async move { take_fence(&host, &executor).await })
    };
    executor.advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    assert!(host.operation_in_progress());
    drop(recovering);
    assert!(!host.operation_in_progress(), "released for a moment");
    executor.advance_clock(std::time::Duration::from_millis(100));
    let taken = waiting.await;
    assert!(host.operation_in_progress(), "now the new tab's open holds it");
    drop(taken);
    assert!(!host.operation_in_progress());
}

/// MCP saves use the headless host, even when the UI project has no file.
#[test]
fn first_save_mcp_never_requires_a_window_or_panel() {
    let fixture = Fixture::new();
    let mut connection = fixture.connection();
    start(&mut connection);
    add(&mut connection, "agent-save");
    assert!(
        fixture
            .host
            .kernel
            .published("test")
            .unwrap()
            .path
            .is_none()
    );
    let first = connection.dispatch("save_project", &json!({})).unwrap();
    let path = std::path::Path::new(first["path"].as_str().unwrap());
    assert!(path.is_file());
    assert!(path.starts_with(&fixture.host.profile));
    let again = connection.dispatch("save_project", &json!({})).unwrap();
    assert_ne!(first["path"], again["path"]);
    assert!(
        fixture
            .host
            .kernel
            .published("test")
            .unwrap()
            .path
            .is_none()
    );
    assert!(!fixture.host.profile.join("document-folders").exists());
}

/// Publication's throwaway activation is not the live project registry. This
/// starts with no extension host and immediately applies the published effect.
#[test]
fn headless_publication_loads_effect_before_first_add_effect() {
    let f = Fixture::new();
    let jobs =
        crate::mcp_services::connect_engine(&f.tools, Default::default(), Default::default())
            .unwrap();
    let mut connection = f.connection();
    let started = connection
        .dispatch(
            "start_session",
            &json!({"prompt":"Synthetic effect proof","includeExtensionStaging":true}),
        )
        .unwrap();
    connection.dispatch("apply_commands", &json!({"commands":[{
        "type":"add_layer", "id":"plate", "layerType":"solid", "content":{"color":"#FF0000","w":320,"h":180}, "name":"Probe plate"
    }]})).unwrap();
    let stage = std::path::PathBuf::from(started["stagingDirectory"].as_str().unwrap());
    let folder = stage.join("cps-probe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("manifest.json"), json!({"id":"cps-probe","name":"Probe","version":"1.0.0","apiVersion":3,"entry":"index.ts","contributes":["effects"]}).to_string()).unwrap();
    std::fs::write(folder.join("index.ts"), "export default function(api){api.effects.register({id:'cps-probe.fx',label:'Probe',group:'Test',params:[{k:'amount',label:'Amount',min:0,max:1,def:0.5}],frag:'o=src(v_uv)*u_amount;'})}").unwrap();
    connection
        .dispatch(
            "publish_extensions",
            &json!({"extensions":[{"id":"cps-probe","action":"created","summary":"Synthetic effect regression"}]}),
        )
        .unwrap();
    assert!(
        f.host
            .kernel
            .state()
            .provider("effects", "cps-probe.fx")
            .is_none(),
        "publication alone creates no persistent realm"
    );
    connection
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"add_effect","target":"plate","effect":"cps-probe.fx"}]}),
        )
        .unwrap();
    assert!(
        f.host
            .kernel
            .state()
            .provider("effects", "cps-probe.fx")
            .is_some()
    );
    let saved = document(&f.host.kernel.published("test").unwrap());
    assert_eq!(saved["layers"][0]["fx"][0]["type"], "cps-probe.fx");
    assert_eq!(saved["layers"][0]["fx"][0]["p"]["amount"]["v"], 0.5);
    let exported = connection.dispatch("save_project", &json!({})).unwrap();
    connection
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    let proof = tempfile::tempdir().unwrap();
    std::fs::copy(
        exported["path"].as_str().unwrap(),
        proof.path().join("saved.pmv"),
    )
    .unwrap();
    let live = f.host.profile.join("extensions/cps-probe");
    for name in ["manifest.json", "index.ts"] {
        std::fs::copy(live.join(name), proof.path().join(name)).unwrap();
    }
    // A new test process has no loaded realm, GPU programs, or live registry.
    for mode in [
        "windowless",
        "windowed-handoff",
        "windowed-pending",
        "removed",
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mcp::tests::extension_effect_reopen_child",
                "--nocapture",
            ])
            .env("PM_EXTENSION_REOPEN_PROOF", proof.path())
            .env("PM_EXTENSION_REOPEN_MODE", mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    drop(connection);
    drop(jobs);
}

#[test]
fn extension_effect_reopen_child() {
    let Some(root) = std::env::var_os("PM_EXTENSION_REOPEN_PROOF") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let mode = std::env::var("PM_EXTENSION_REOPEN_MODE").unwrap();
    let f = Fixture::new();
    if mode != "removed" {
        let folder = f.host.profile.join("extensions/cps-probe");
        std::fs::create_dir_all(&folder).unwrap();
        for name in ["manifest.json", "index.ts"] {
            std::fs::copy(root.join(name), folder.join(name)).unwrap();
        }
    }
    f.host
        .open(
            "restored".into(),
            SessionSource::File(root.join("saved.pmv")),
        )
        .unwrap();
    let published = f.host.kernel.published("restored").unwrap();
    let before = document(&published)["layers"][0]["fx"].clone();
    assert_eq!(before[0]["type"], "cps-probe.fx");
    let jobs =
        crate::mcp_services::connect_engine(&f.tools, Default::default(), Default::default())
            .unwrap();
    let mut window_host = None;
    if mode.starts_with("windowed-") {
        // A window's owner starts before the UI publishes its next resource
        // pin. Deliberately leave an old ready pin to guard that exact race.
        window_host = Some(pm_compat_runner::host::ExtensionHost::start(
            pm_compat_runner::host::HostConfig {
                kernel: f.host.kernel.clone(),
                project: "restored".into(),
                roots: crate::extensions::roots(&f.host.profile),
                cache: f.host.profile.join("window-extension-cache"),
                owner: None,
                text: None,
                frames: None,
                rasters: None,
                clipboard: None,
                vars: None,
                wake: Arc::new(|| {}),
            },
        ));
        crate::mcp_services::set_resources(
            published.snapshot.doc_id,
            Some(Arc::new(pm_engine_service::resources::ResourceSnapshot {
                generation: 1,
                pending: mode == "windowed-pending",
                ..Default::default()
            })),
        );
    }
    // This is the first render's resource request, with no prior effect edit.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let resources = jobs
        .resources(&published, true, &|| std::time::Instant::now() >= deadline)
        .unwrap();
    if mode.starts_with("windowed-") {
        assert!(
            resources
                .definitions
                .iter()
                .any(|d| d.id.as_str() == "cps-probe.fx")
        );
        assert!(
            resources.generation > 1,
            "the stale ready pin was refreshed"
        );
    }
    let mut renderer = pm_render::Renderer::headless().unwrap();
    renderer.set_bypass_missing_effects(true);
    assert!(
        renderer
            .set_document_effects(&published.snapshot.document, &resources.definitions)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        renderer
            .missing_effects()
            .iter()
            .any(|e| e.effect == "cps-probe.fx"),
        mode == "removed"
    );
    let request = pm_render::RenderRequest::sized(
        pm_protocol::common::Size::new(32, 18),
        pm_protocol::color::Precision::Float16,
    );
    let frame = renderer
        .render(
            &published.snapshot.document,
            "c",
            pm_protocol::common::Ticks::new(0).unwrap(),
            &request,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let effected = renderer.read_rgba(&frame).unwrap();
    let mut plain = document(&published);
    plain["layers"][0]["fx"] = json!([]);
    let plain = pm_model::Document::new(Some(model(&plain)), 1);
    let frame = renderer
        .render(
            &plain,
            "c",
            pm_protocol::common::Ticks::new(0).unwrap(),
            &request,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    let plain = renderer.read_rgba(&frame).unwrap();
    let difference = effected
        .iter()
        .zip(&plain)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    if mode == "removed" {
        assert!(
            difference < 0.002,
            "missing effect must bypass: {difference}"
        );
    } else {
        assert!(
            difference > 0.05,
            "first frame must apply saved effect: {difference}"
        );
    }
    assert_eq!(
        before,
        document(&f.host.kernel.published("restored").unwrap())["layers"][0]["fx"]
    );
    crate::mcp_services::set_resources(published.snapshot.doc_id, None);
    drop(window_host);
    drop(jobs);
}

#[test]
fn headless_v4_publication_registers_effects_in_the_live_catalog() {
    let f = Fixture::new();
    let jobs =
        crate::mcp_services::connect_engine(&f.tools, Default::default(), Default::default())
            .unwrap();
    let mut connection = f.connection();
    let started = connection
        .dispatch(
            "start_session",
            &json!({"prompt":"Synthetic native effect","includeExtensionStaging":true}),
        )
        .unwrap();
    add(&mut connection, "plate");
    let folder = std::path::PathBuf::from(started["stagingDirectory"].as_str().unwrap())
        .join("native-probe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("manifest.json"), json!({"id":"native-probe","name":"Native Probe","version":"1.0.0","apiVersion":4,"entry":"index.ts","contributes":["effects"]}).to_string()).unwrap();
    std::fs::write(folder.join("index.ts"), "export async function mount(env){ env.effects.register({id:'native-probe.fx',label:'Native Probe',group:'Test',params:[{k:'amount',label:'Amount',min:0,max:1,def:0.5}],frag:'o=src(v_uv)*u_amount;'}); const root=env.root(); await root.render(null); return {root,dispose:()=>root.dispose()}; }").unwrap();
    connection
        .dispatch(
            "publish_extensions",
            &json!({"extensions":[{"id":"native-probe","action":"created","summary":"Synthetic native effect regression"}]}),
        )
        .unwrap();
    connection.dispatch("apply_commands", &json!({"commands":[{"type":"add_effect","target":"plate","effect":"native-probe.fx"}]})).unwrap();
    let published = f.host.kernel.published("test").unwrap();
    let resources = jobs.resources(&published, true, &|| false).unwrap();
    assert!(
        resources
            .definitions
            .iter()
            .any(|d| d.id.as_str() == "native-probe.fx")
    );
    let mut renderer = pm_render::Renderer::headless().unwrap();
    assert!(
        renderer
            .set_document_effects(&published.snapshot.document, &resources.definitions)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        document(&published)["layers"][0]["fx"][0]["type"],
        "native-probe.fx"
    );
    connection
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    drop(connection);
    drop(jobs);
}

#[test]
fn agent_completion_bookkeeping_reuses_the_callers_operation_guard() {
    let fixture = Fixture::new();
    let guard = fixture.host.try_operation().unwrap();
    super::completion_on(
        &fixture.host,
        &fixture.tools.automation,
        "begin",
        "test",
        "run",
        &json!({"threadId":"thread"}),
    )
    .unwrap();
    super::completion_on(
        &fixture.host,
        &fixture.tools.automation,
        "bindChild",
        "test",
        "child",
        &json!({"rootRunId":"run"}),
    )
    .unwrap();
    let report = super::completion_on(
        &fixture.host,
        &fixture.tools.automation,
        "verify",
        "test",
        "run",
        &json!({}),
    )
    .unwrap();
    assert_eq!(report["changed"], false);
    assert!(fixture.host.operation_in_progress());
    let error = super::completion_on(
        &fixture.host,
        &fixture.tools.automation,
        "undo",
        "test",
        "run",
        &report,
    )
    .unwrap_err();
    assert!(error.contains("project is changing"));
    super::completion_on(
        &fixture.host,
        &fixture.tools.automation,
        "keep",
        "test",
        "run",
        &report,
    )
    .unwrap();
    assert!(
        fixture.host.operation_in_progress(),
        "bookkeeping must not release the caller's lease"
    );
    drop(guard);
    assert!(!fixture.host.operation_in_progress());
}

#[test]
fn agent_native_plan_preserves_a_concurrent_editors_history_group() {
    let fixture = Fixture::new();
    super::completion_on(
        &fixture.host,
        &fixture.tools.automation,
        "begin",
        "test",
        "run",
        &json!({}),
    )
    .unwrap();
    let editor = Caller::realm(42, 1, Origin::Agent);
    fixture
        .host
        .kernel
        .call_blocking(
            "test",
            Request::new(
                editor,
                HistoryCall::Begin {
                    label: "Independent editor gesture".into(),
                },
            ),
        )
        .unwrap();
    let revision = fixture.host.kernel.published("test").unwrap().revision();
    let result=super::completion_on(&fixture.host,&fixture.tools.automation,"apply","test","run",&json!({"revision":revision,"commands":[{"type":"add_layer","id":"agent","layerType":"solid"}]})).unwrap();
    assert_eq!(result["changed"], true);
    assert!(
        result["verificationError"]
            .as_str()
            .unwrap()
            .contains("alongside other edits")
    );
    fixture
        .host
        .kernel
        .call_blocking(
            "test",
            Request::new(editor, HistoryCall::Commit { label: None }),
        )
        .unwrap();
    let before = document(&fixture.host.kernel.published("test").unwrap());
    assert_eq!(before["layers"].as_array().unwrap().len(), 1);
    assert!(
        super::completion_on(
            &fixture.host,
            &fixture.tools.automation,
            "undo",
            "test",
            "run",
            &result
        )
        .unwrap_err()
        .contains("Other edits")
    );
    assert_eq!(
        document(&fixture.host.kernel.published("test").unwrap()),
        before
    );
}

#[test]
fn aero_repairs_a_compile_error_and_loads_the_published_extension() {
    use pm_agents::{
        Access, Event, Provider, RunResult,
        aero::{self, Model, Options, Reply},
        launch::Request as Launch,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    let fixture = Fixture::new();
    {
        let mut session = fixture.connection();
        start(&mut session);
        session.dispatch("apply_commands", &json!({"commands":[
            {"type":"add_layer","id":"plate","name":"Plate","layerType":"solid","from":0,"duration":2}
        ]})).unwrap();
        session.dispatch("finish_session", &json!({"commit":true})).unwrap();
    }
    let jobs = crate::mcp_services::connect_engine(&fixture.tools, Default::default(), Default::default()).unwrap();
    let scope = fixture.host.profile.join("aero/sessions/extension");
    let host = fixture.tools.automation.clone();
    let source = include_str!("../../../js/compat/api-pack/samples/gradient-tint/index.ts");
    let manifest = include_str!("../../../js/compat/api-pack/samples/gradient-tint/manifest.json");
    let step = AtomicUsize::new(0);
    let model = Model::fake(move |request| {
        let tool = |name: &str, op: &str, args: Value| Reply::Tool {
            name: name.into(), args: json!({"op":op,"args":args}),
        };
        match step.fetch_add(1, Ordering::Relaxed) {
            0 => tool("files","write",json!({"path":"gradient-tint/manifest.json","text":manifest})),
            1 => tool("files","write",json!({"path":"gradient-tint/index.ts","text":"const valid = 1;\nexport default function ( {"})),
            2 => tool("extension","check",json!({"id":"gradient-tint"})),
            3 => {
                assert!(String::from_utf8_lossy(request).contains("index.ts:2:"));
                tool("files","write",json!({"path":"gradient-tint/index.ts","text":source}))
            }
            4 => tool("extension","check",json!({"id":"gradient-tint"})),
            5 => tool("extension","publish",json!({"extensions":[{"id":"gradient-tint","action":"created"}]})),
            _ => Reply::Text("Published the repaired gradient tint.".into()),
        }
    });
    let report: aero::Report = Arc::new(|_| {});
    let handle = aero::start(Launch {
        run: "extension".into(), prompt: "Make a gradient tint effect.".into(),
        provider:Provider::Aero, model:None, effort:None, access:Access::Project,
        approval:Default::default(), computer_consent:false, workspace:scope.clone(),
        images:Vec::new(), role:None, files:Vec::new(), project:Some("test".into()),
    }, "extension".into(), scope, Options {
        model, wall:Duration::from_secs(45), report:Some(report),
        ..Options::development()
    }, move |scope| {
        pm_mcp::session::aero_tools::Tools::open(host,"test",None,scope,Access::Project,None)
            .map(|tools| Box::new(tools) as Box<dyn aero::Session>)
            .map_err(|e| e.to_string())
    });
    let deadline = Instant::now() + Duration::from_secs(50);
    let result = loop {
        match handle.try_event() {
            Ok(Event::Completed { result,.. }) => break result,
            Ok(Event::Trace { value, .. }) if value["kind"] == "tool-end" => eprintln!("{value}"),
            Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline =>
                std::thread::sleep(Duration::from_millis(5)),
            event => panic!("Aero did not complete: {event:?}"),
        }
    };
    assert_eq!(result,RunResult::success("Published the repaired gradient tint.".into(),Access::Project));
    let published = fixture.host.kernel.published("test").unwrap();
    let resources = jobs.resources(&published, true, &|| false).unwrap();
    assert!(resources.definitions.iter().any(|d| d.id.as_str() == "gradient-tint.effect"));
    drop(handle);
    drop(jobs);
}
