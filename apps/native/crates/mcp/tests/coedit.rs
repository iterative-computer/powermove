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
