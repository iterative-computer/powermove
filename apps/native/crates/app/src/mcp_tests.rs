use super::*;
struct Fixture {
    _root: tempfile::TempDir,
    actor: Option<KernelActor>,
    host: Arc<Host>,
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
            _root: root,
            actor: Some(actor),
            host,
        }
    }
    fn connection(&self) -> Connection {
        Connection::new(self.host.clone())
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
    assert_eq!(
        c.dispatch("get_project_state", &json!({"indexOnly":true}))
            .unwrap_err()
            .code,
        "revision_conflict"
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
fn live_app_mcp_exports_unique_packed_files_without_adopting_or_overwriting_source() {
    let f = Fixture::new();
    let source = f._root.path().join("working.pmv");
    f.host
        .save(f.host.current().unwrap(), Some(source.clone()))
        .unwrap();
    let original = std::fs::read(&source).unwrap();
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
    assert_eq!(std::fs::read(source).unwrap(), original);
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
    let saved = f.host.export(current, before.snapshot.doc_id).unwrap();
    let opened = pm_kernel::open_project(
        std::path::Path::new(saved["path"].as_str().unwrap()),
        Default::default(),
    )
    .unwrap();
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
    let mut connection = f.connection();
    start(&mut connection);
    assert!(f.host.kernel.close("test").wait().unwrap());
    f.host.kernel.open("test",OpenSource::Untitled(model(&json!({"id":"test","name":"Replacement","compId":"c","w":320,"h":180,"fps":30,"dur":2,"layers":[],"assets":{},"comps":{}})))).wait().unwrap().unwrap();
    let before = f.host.kernel.published("test").unwrap();
    assert!(
        connection
            .call(
                "test",
                HistoryCall::Begin {
                    label: "Retired request".into()
                }
            )
            .is_err()
    );
    let after = f.host.kernel.published("test").unwrap();
    assert_eq!(after.revision(), before.revision());
    assert!(after.entry.is_none());
    assert_eq!(
        after.snapshot.document.to_value(),
        before.snapshot.document.to_value()
    );
}
