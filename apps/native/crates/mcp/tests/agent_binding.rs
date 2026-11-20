//! An agent run works in the project its thread was started from, whichever
//! window is in front (PM-550); an application-window run is told how to get
//! a project and keeps the one it creates or opens (PM-549).
use pm_mcp::{Automation, McpSession};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
fn ok(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn layers(host: &Arc<Mutex<Automation>>, id: &str) -> usize {
    host.lock().unwrap().read_document(id).unwrap()["layers"]
        .as_array()
        .unwrap()
        .len()
}
fn add_layer(session: &mut McpSession, id: &str) -> Value {
    session.call(
        "apply_commands",
        json!({"commands":[{"type":"add_layer","id":id,"layerType":"shape","name":"Synthetic"}]}),
    )
}
/// A running app's host with editor windows 1 and 2 showing projects A and B.
fn two_windows() -> (
    tempfile::TempDir,
    pm_kernel::KernelActor,
    Arc<pm_mcp::app_host::Host>,
    Automation,
    String,
    String,
) {
    use pm_kernel::{
        KernelActor, KernelConfig,
        locks::OpenDecision,
        persistence::DiskFormat,
        session::{ShellCall, ShellReply},
    };
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().to_path_buf();
    let actor = KernelActor::spawn(KernelConfig::default());
    assert!(matches!(
        actor
            .handle()
            .shell(ShellCall::AttachLibrary {
                profile: profile.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None
            })
            .wait()
            .unwrap()
            .unwrap(),
        ShellReply::Library(OpenDecision::Writable)
    ));
    let app = pm_mcp::app_host::Host::new(actor.handle(), profile.clone());
    let _one = app.attach_window(1);
    let _two = app.attach_window(2);
    let mut host = Automation::attached(app.clone(), profile.join("exports"));
    let mut create = |name: &str| {
        host.create_project(&json!({ "name": name })).unwrap()["projectId"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let a = create("Synthetic A");
    let b = create("Synthetic B");
    assert_eq!(app.claim_placed(1).unwrap().project, a);
    assert_eq!(app.claim_placed(2).unwrap().project, b);
    (root, actor, app, host, a, b)
}
fn agent_session(
    mut host: Automation,
    project: Option<String>,
) -> (Arc<Mutex<Automation>>, McpSession) {
    host.agent_grants = Some(Arc::new(|token| {
        (token == "synthetic-grant").then(|| ("synthetic-run".into(), "project".into()))
    }));
    host.agent_projects = Some(Arc::new(move |token| {
        (token == "synthetic-grant")
            .then(|| project.clone())
            .flatten()
    }));
    let host = Arc::new(Mutex::new(host));
    let mut session = McpSession::new(host.clone());
    session.set_agent_run("synthetic-grant");
    (host, session)
}
#[test]
fn a_run_from_project_a_edits_a_while_project_bs_window_is_key() {
    let (_root, _actor, app, host, a, b) = two_windows();
    app.activate_window(2);
    assert_eq!(app.current().unwrap().project, b);
    let (host, mut session) = agent_session(host, Some(a.clone()));
    let started = ok(session.call("start_session", json!({"prompt":"Synthetic"})));
    assert_eq!(started["projectId"], a.as_str(), "{started}");
    ok(add_layer(&mut session, "bound"));
    assert_eq!(layers(&host, &a), 1);
    assert_eq!(layers(&host, &b), 0);
    // Starting there never brought A's window to the front.
    assert_eq!(app.current().unwrap().project, b);
    session.finish(true).unwrap();
}
#[test]
fn a_run_from_project_a_edits_a_while_an_empty_window_is_key() {
    let (_root, _actor, app, host, a, b) = two_windows();
    let _three = app.attach_window(3);
    app.activate_window(3);
    assert!(app.current().is_none());
    let (host, mut session) = agent_session(host, Some(a.clone()));
    let started = ok(session.call("start_session", json!({"prompt":"Synthetic"})));
    assert_eq!(started["projectId"], a.as_str(), "{started}");
    ok(add_layer(&mut session, "bound"));
    assert_eq!(layers(&host, &a), 1);
    assert_eq!(layers(&host, &b), 0);
    session.finish(true).unwrap();
}
#[test]
fn a_run_whose_project_closed_is_told_so_instead_of_editing_another() {
    let (_root, _actor, app, host, a, b) = two_windows();
    app.activate_window(2);
    let (host, mut session) = agent_session(host, Some("synthetic-closed-project".into()));
    let refused = session.call("start_session", json!({"prompt":"Synthetic"}));
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(
        refused["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("This thread's project isn't open"),
        "{refused}"
    );
    assert_eq!(layers(&host, &a), 0);
    assert_eq!(layers(&host, &b), 0);
}
#[test]
fn an_app_run_without_a_project_keeps_the_key_window_default() {
    let (_root, _actor, app, host, a, b) = two_windows();
    app.activate_window(2);
    let (host, mut session) = agent_session(host, None);
    let started = ok(session.call("start_session", json!({"prompt":"Synthetic"})));
    assert_eq!(started["projectId"], b.as_str(), "{started}");
    ok(add_layer(&mut session, "front"));
    assert_eq!(layers(&host, &a), 0);
    assert_eq!(layers(&host, &b), 1);
    session.finish(true).unwrap();
}
fn app_session() -> (tempfile::TempDir, Arc<Mutex<Automation>>, McpSession, Value) {
    let root = tempfile::tempdir().unwrap();
    let host = Automation::new(root.path().into(), root.path().join("exports")).unwrap();
    let (host, mut session) = agent_session(host, None);
    let started = ok(session.call(
        "start_session",
        json!({"prompt":"Synthetic","context":"app"}),
    ));
    (root, host, session, started)
}
#[test]
fn an_app_session_that_creates_a_project_can_edit_it_at_once() {
    let (_root, host, mut session, _) = app_session();
    let created = ok(session.call("create_project", json!({"name":"Synthetic created"})));
    let id = created["projectId"].as_str().unwrap().to_owned();
    ok(add_layer(&mut session, "first"));
    assert_eq!(layers(&host, &id), 1);
    ok(session.call("get_project_state", json!({"indexOnly":true})));
    assert_eq!(
        ok(session.call("finish_session", json!({"commit":true})))["changed"],
        true
    );
    assert_eq!(layers(&host, &id), 1);
}
#[test]
fn an_app_session_that_creates_through_manage_project_can_edit_it_at_once() {
    let (_root, host, mut session, _) = app_session();
    let created = ok(session.call(
        "manage_project",
        json!({"operation":"create","name":"Synthetic managed"}),
    ));
    let id = created["projectId"].as_str().unwrap().to_owned();
    ok(add_layer(&mut session, "first"));
    assert_eq!(layers(&host, &id), 1);
    ok(session.call("finish_session", json!({"commit":true})));
    assert_eq!(layers(&host, &id), 1);
}
#[test]
fn an_app_session_that_opens_a_project_can_edit_it_at_once() {
    let (_root, host, mut session, _) = app_session();
    let id = host
        .lock()
        .unwrap()
        .create_project(&json!({"name":"Synthetic existing"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(session.call("open_project", json!({"projectId":id})));
    ok(add_layer(&mut session, "first"));
    assert_eq!(layers(&host, &id), 1);
    ok(session.call("finish_session", json!({"commit":true})));
}
#[test]
fn the_app_session_reply_says_how_to_get_a_project() {
    let (_root, _host, _session, started) = app_session();
    let instructions = started["instructions"].as_str().unwrap();
    assert!(
        instructions.contains(
            "No project is open. Call create_project (or open_project), then you can add layers and use render_frames."
        ),
        "{instructions}"
    );
    // Composition tools are refused until a project is attached: not advertised.
    for refused in ["get_project_state", "apply_commands", "compositionId"] {
        assert!(!instructions.contains(refused), "{refused}: {instructions}");
    }
}
#[test]
fn an_app_session_refuses_composition_tools_until_it_has_a_project() {
    let (_root, _host, mut session, _) = app_session();
    let refused = session.call("get_project_state", json!({"indexOnly":true}));
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(
        refused["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("This agent has no project attached"),
        "{refused}"
    );
}
