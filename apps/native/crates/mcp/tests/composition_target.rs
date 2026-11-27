//! Composition targeting must never publish editor navigation.
use pm_kernel::{CompCommand, Origin, commands::comps::NewComp};
use pm_mcp::{Automation, CaptureRequest, EngineHook, McpSession, Result, services::FrozenProject};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct Capture;
impl EngineHook for Capture {
    fn capture(&self, _: &CaptureRequest) -> Result<Value> {
        panic!("must use frozen target")
    }
    fn capture_snapshot(&self, _: &CaptureRequest, frozen: &FrozenProject) -> Result<Value> {
        assert_eq!(frozen.project["compId"], "pre");
        assert_eq!(frozen.project["w"], 640);
        assert_eq!(frozen.project["layers"][0]["id"], "inside");
        Ok(json!({"images":["data:image/png;base64,AAAA"]}))
    }
}
struct Fixture {
    root: std::path::PathBuf,
    host: Arc<Mutex<Automation>>,
    id: String,
    actor: Option<pm_kernel::KernelActor>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_actor(false)
    }
    fn with_actor(attached: bool) -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("composition-test"));
        std::fs::create_dir_all(&root).unwrap();
        let actor = attached.then(|| {
            pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig {
                journal_root: Some(root.join("journals")),
                backup_root: Some(root.join("backups")),
                ..Default::default()
            })
        });
        let mut host = if let Some(actor) = &actor {
            let reply = actor
                .handle()
                .shell(pm_kernel::session::ShellCall::AttachLibrary {
                    profile: root.clone(),
                    format: pm_kernel::persistence::DiskFormat::V1_2,
                    electron_profile: None,
                })
                .wait()
                .unwrap()
                .unwrap();
            assert!(matches!(reply, pm_kernel::session::ShellReply::Library(_)));
            Automation::attached(
                pm_mcp::app_host::Host::new(actor.handle(), root.clone()),
                root.join("exports"),
            )
        } else {
            Automation::new(root.clone(), root.join("exports")).unwrap()
        };
        let id = host
            .create_project(&json!({"name":"Synthetic targeting"}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .to_owned();
        let rev = host.revision(&id).unwrap();
        host.apply_commands(
            &id,
            vec![
                CompCommand::Create {
                    comp_id: "pre".into(),
                    settings: NewComp {
                        name: Some("Native UI".into()),
                        width: Some(640.),
                        ..Default::default()
                    },
                }
                .into(),
            ],
            "Fixture",
            Origin::Mcp,
            rev,
            1,
        )
        .unwrap();
        if let Some(actor) = &actor {
            let mut ui = pm_kernel::state::UiState::default();
            ui.transport = pm_model::json::parse(r#"{"time":3.25,"playing":false}"#).unwrap();
            ui.timeline = pm_model::json::parse(r#"{"pps":75,"scrollY":125,"scrollT":2}"#).unwrap();
            actor
                .handle()
                .state_transaction(pm_kernel::state::StateTransaction {
                    ops: vec![pm_kernel::state::StateOp::Ui {
                        project: id.clone(),
                        next: ui,
                    }],
                    ..Default::default()
                })
                .wait()
                .unwrap()
                .unwrap();
        }
        host.engine = Some(Arc::new(Capture));
        Self {
            root,
            host: Arc::new(Mutex::new(host)),
            id,
            actor,
        }
    }
    fn session(&self) -> McpSession {
        let mut s = McpSession::new(self.host.clone());
        s.dispatch(
            "start_session",
            &json!({"prompt":"Synthetic comp targeting"}),
        )
        .unwrap();
        s
    }
    fn document(&self) -> Value {
        self.host.lock().unwrap().read_document(&self.id).unwrap()
    }
    fn edit(&self, s: &mut McpSession) {
        s.dispatch("apply_commands", &json!({"compositionId":"pre","commands":[{"type":"add_layer","id":"inside","name":"Inside","layerType":"solid"}]})).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.host.lock().unwrap().close_headless();
        if let Some(actor) = self.actor.take() {
            actor.shutdown();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn targeted_read_edit_selection_render_and_undo_preserve_editor() {
    exercise_editor(false);
}
#[test]
fn attached_actor_targeting_preserves_playhead_scroll_and_undo() {
    exercise_editor(true);
}
fn exercise_editor(attached: bool) {
    let f = Fixture::with_actor(attached);
    let mut s = f.session();
    s.dispatch(
        "apply_commands",
        &json!({"commands":[{"type":"add_layer","id":"outside","layerType":"solid"}]}),
    )
    .unwrap();
    s.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    let before = f.document();
    let selection = f.host.lock().unwrap().selection(&f.id).unwrap();
    let ui = f.host.lock().unwrap().state().ui.clone();
    let workspace = f.host.lock().unwrap().view(&f.id).unwrap().workspace;
    let mut s = f.session();
    let read = s
        .dispatch("get_project_state", &json!({"compositionId":"pre"}))
        .unwrap();
    assert_eq!(read["composition"]["name"], "Native UI");
    assert_eq!(read["compositions"].as_array().unwrap().len(), 2);
    assert_eq!(read["layerCount"], 0);
    f.edit(&mut s);
    s.dispatch(
        "select_layers",
        &json!({"compositionId":"pre","layerIds":["inside"]}),
    )
    .unwrap();
    let targeted = s
        .dispatch("get_project_state", &json!({"compositionId":"pre"}))
        .unwrap();
    assert_eq!(targeted["layers"][0]["id"], "inside");
    assert_eq!(targeted["selection"]["layers"], json!(["inside"]));
    s.dispatch("render_frames", &json!({"compositionId":"pre","times":[0]}))
        .unwrap();
    assert_eq!(
        s.dispatch("get_project_state", &json!({})).unwrap()["layers"][0]["id"],
        "outside"
    );
    assert_eq!(f.document()["compId"], before["compId"]);
    assert_eq!(f.host.lock().unwrap().selection(&f.id).unwrap(), selection);
    assert_eq!(f.host.lock().unwrap().state().ui, ui);
    assert_eq!(
        f.host.lock().unwrap().view(&f.id).unwrap().workspace,
        workspace
    );
    s.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    f.host.lock().unwrap().undo(&f.id, Origin::Mcp, 77).unwrap();
    assert_eq!(
        f.document()["comps"]["pre"]["layers"],
        before["comps"]["pre"]["layers"]
    );
}

#[test]
fn disconnect_restores_target_and_keeps_open_comp() {
    let f = Fixture::new();
    let before = f.document();
    {
        let mut s = f.session();
        f.edit(&mut s);
        s.close().unwrap();
    }
    let mut after = f.document();
    after["revision"] = before["revision"].clone();
    assert_eq!(after, before);
}

#[test]
fn invalid_target_never_falls_back_and_scope_clears_on_error() {
    let f = Fixture::new();
    let mut s = f.session();
    assert!(s.dispatch("apply_commands", &json!({"compositionId":"missing","commands":[{"type":"add_layer","layerType":"solid"}]})).is_err());
    assert!(
        s.dispatch("get_project_state", &json!({"compositionId":42}))
            .is_err()
    );
    assert_eq!(
        s.dispatch("get_project_state", &json!({})).unwrap()["composition"]["id"],
        f.document()["id"]
    );
    for name in pm_mcp::catalog::COMPOSITION_TOOLS {
        assert_eq!(
            pm_mcp::catalog::spec(name).unwrap()["inputSchema"]["properties"]["compositionId"]["type"],
            "string"
        );
    }
}

#[test]
fn attached_actor_disconnect_restores_target() {
    let f = Fixture::with_actor(true);
    let before = f.document();
    let mut s = f.session();
    f.edit(&mut s);
    s.close().unwrap();
    let mut after = f.document();
    after["revision"] = before["revision"].clone();
    assert_eq!(after, before);
}

#[test]
fn targeted_edits_save_and_remain_visible_after_navigation() {
    let f = Fixture::new();
    let mut s = f.session();
    f.edit(&mut s);
    s.dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    let saved = f.host.lock().unwrap().save_project(&f.id).unwrap();
    let opened = pm_kernel::project_io::open_project(
        std::path::Path::new(saved["path"].as_str().unwrap()),
        Default::default(),
    )
    .unwrap();
    let reopened = pm_kernel::DocumentStore::open(opened);
    let value = reopened.snapshot().document.to_value().unwrap();
    let value: Value = serde_json::from_str(&pm_model::json::to_canonical_string(&value)).unwrap();
    assert_eq!(value["comps"]["pre"]["layers"][0]["id"], "inside");
    // The canonical stored project contains the precomp edits, and opening the
    // comp through the ordinary kernel path reveals those exact layers.
    let mut h = f.host.lock().unwrap();
    let rev = h.revision(&f.id).unwrap();
    h.apply_commands(
        &f.id,
        vec![
            CompCommand::Open {
                comp_id: "pre".into(),
            }
            .into(),
        ],
        "Navigate fixture",
        Origin::Mcp,
        rev,
        9,
    )
    .unwrap();
    assert_eq!(h.read_document(&f.id).unwrap()["layers"][0]["id"], "inside");
}
