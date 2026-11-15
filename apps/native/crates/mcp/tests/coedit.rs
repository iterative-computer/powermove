//! Synthetic, headless co-editing regressions.
use pm_mcp::{Automation, McpSession};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
fn ok(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn fixture() -> (
    std::path::PathBuf,
    Arc<Mutex<Automation>>,
    McpSession,
    String,
) {
    let root = std::env::temp_dir().join(pm_kernel::new_id("coedit"));
    let mut h = Automation::new(root.clone(), root.join("exports")).unwrap();
    let id = h
        .create_project(&json!({"name":"Synthetic coedit"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let h = Arc::new(Mutex::new(h));
    let mut s = McpSession::new(h.clone());
    ok(s.call("start_session", json!({"prompt":"Synthetic coedit"})));
    (root, h, s, id)
}
#[test]
fn agent_selection_and_creates_never_change_interface_selection() {
    let (root, h, mut s, id) = fixture();
    ok(s.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"a","layerType":"shape","name":"A"}]}),
    ));
    assert!(
        h.lock()
            .unwrap()
            .view(&id)
            .unwrap()
            .selection
            .layers()
            .is_empty()
    );
    let result = ok(s.call("select_layers", json!({"layerIds":["a"]})));
    assert_eq!(result["scope"], "agent");
    assert!(
        h.lock()
            .unwrap()
            .view(&id)
            .unwrap()
            .selection
            .layers()
            .is_empty()
    );
    ok(s.call("apply_commands",json!({"commands":[{"type":"set_property","target":"$selection","path":"opacity","value":40}]})));
    s.finish(true).unwrap();
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn session_stays_bound_when_front_project_changes() {
    let (root, h, mut s, id) = fixture();
    let other = h
        .lock()
        .unwrap()
        .create_project(&json!({"name":"Other"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(id, other);
    ok(s.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"bound","layerType":"shape","name":"Bound"}]}),
    ));
    assert_eq!(
        h.lock().unwrap().read_document(&id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        h.lock().unwrap().read_document(&other).unwrap()["layers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    s.finish(true).unwrap();
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn presence_is_project_bound_and_never_stalls_a_writer() {
    let (root, h, mut s, id) = fixture();
    let rx = h.lock().unwrap().subscribe_presence();
    for i in 0..12 {
        ok(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":format!("a{i}"),"layerType":"shape","name":"Synthetic"}]})));
    }
    let events = rx.try_iter().collect::<Vec<_>>();
    assert_eq!(events.len(), 6);
    assert!(events.iter().all(|e| e.project == id));
    s.finish(true).unwrap();
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn a_deleted_agent_target_returns_target_removed() {
    let (root, h, mut s, id) = fixture();
    ok(s.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"gone","layerType":"shape","name":"Gone"}]}),
    ));
    {
        let mut h = h.lock().unwrap();
        let revision = h.revision(&id).unwrap();
        h.apply_commands(
            &id,
            vec![
                pm_kernel::LayerCommand::Delete {
                    layer_ids: vec!["gone".into()],
                }
                .into(),
            ],
            "User delete",
            pm_kernel::Origin::Mcp,
            revision,
            7,
        )
        .unwrap();
    }
    let result = s.call(
        "apply_commands",
        json!({"commands":[{"type":"set_property","target":"gone","path":"opacity","value":30}]}),
    );
    assert_eq!(result["isError"], true);
    assert!(result.to_string().contains("target_removed"), "{result}");
    // Interleaved delete is preserved on disconnect.
    let _ = s.finish(true);
    drop(s);
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}

fn attached_fixture() -> (
    std::path::PathBuf,
    Arc<Mutex<Automation>>,
    pm_kernel::KernelActor,
    McpSession,
    String,
) {
    use pm_kernel::{
        KernelActor, KernelConfig,
        locks::OpenDecision,
        persistence::DiskFormat,
        session::{ShellCall, ShellReply},
    };
    let root = std::env::temp_dir().join(pm_kernel::new_id("coedit-attached"));
    std::fs::create_dir_all(&root).unwrap();
    let actor = KernelActor::spawn(KernelConfig::default());
    assert!(matches!(
        actor
            .handle()
            .shell(ShellCall::AttachLibrary {
                profile: root.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None
            })
            .wait()
            .unwrap()
            .unwrap(),
        ShellReply::Library(OpenDecision::Writable)
    ));
    let app = pm_mcp::app_host::Host::new(actor.handle(), root.clone());
    let mut host = Automation::attached(app, root.join("exports"));
    let id = host
        .create_project(&json!({"name":"Synthetic held write"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let host = Arc::new(Mutex::new(host));
    let mut session = McpSession::new(host.clone());
    ok(session.call("start_session", json!({"prompt":"Synthetic hold"})));
    ok(session.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":"L","layerType":"shape","name":"Synthetic"}]}),
    ));
    (root, host, actor, session, id)
}
#[test]
fn a_live_interface_lease_holds_an_agent_write_then_lands_after_release() {
    use pm_kernel::{Caller, GestureCall, Reply, Request};
    let (root, host, actor, mut session, id) = attached_fixture();
    let kernel = actor.handle();
    let ack = kernel
        .call(
            &id,
            Request::new(
                Caller::native(),
                GestureCall::Begin {
                    label: "User drag".into(),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let Reply::Gesture(gesture) = ack.reply else {
        panic!("expected gesture");
    };
    let release = kernel.clone();
    let project = id.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        release
            .call(
                &project,
                Request::new(Caller::native(), GestureCall::Commit { gesture }),
            )
            .wait()
            .unwrap()
            .unwrap();
    });
    let result = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap();
    thread.join().unwrap();
    assert_eq!(result["status"], "applied", "{result}");
    session.finish(true).unwrap();
    assert_eq!(
        host.lock().unwrap().read_document(&id).unwrap()["layers"][0]["p"]["opacity"]["v"],
        40
    );
    drop(session);
    drop(host);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn typing_holds_after_four_seconds_and_user_wins_at_release() {
    use pm_kernel::{Call, Caller, Command, Request, WriteBody};
    let (root, host, actor, mut session, id) = attached_fixture();
    pm_mcp::coedit::touch_field(&id, "synthetic-opacity", true);
    let result = session
        .dispatch(
            "apply_commands",
            &json!({"commands":[{"type":"set_property","target":"L","path":"opacity","value":40}]}),
        )
        .unwrap();
    assert_eq!(result["status"], "held", "{result}");
    actor
        .handle()
        .call(
            &id,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("User value".into()),
                    body: WriteBody::Commands(vec![Command::SetStaticOpacity {
                        layer_id: "L".into(),
                        value: 33.,
                    }]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    pm_mcp::coedit::touch_field(&id, "synthetic-opacity", false);
    let result = session
        .dispatch("finish_session", &json!({"commit":true}))
        .unwrap();
    assert_eq!(result["coeditNote"]["status"], "dropped", "{result}");
    assert_eq!(
        host.lock().unwrap().read_document(&id).unwrap()["layers"][0]["p"]["opacity"]["v"],
        33
    );
    drop(session);
    drop(host);
    drop(actor);
    std::fs::remove_dir_all(root).unwrap();
}
