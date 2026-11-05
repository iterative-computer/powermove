use pm_mcp::{Automation, McpSession, catalog};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Fixture {
    root: PathBuf,
    host: Arc<Mutex<Automation>>,
    id: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(pm_kernel::new_id("pm-scene3d-mcp"));
        let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
        let id = host
            .create_project(&json!({"name":"Studio fixture","duration":10}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .into();
        Self {
            root,
            host: Arc::new(Mutex::new(host)),
            id,
        }
    }
    fn session(&self) -> McpSession {
        let mut s = McpSession::new(self.host.clone());
        ok(s.call("start_session", json!({"prompt":"Synthetic 3D contract"})));
        s
    }
    fn read(&self) -> Value {
        self.host.lock().unwrap().read_document(&self.id).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn ok(v: Value) -> Value {
    assert_eq!(v["isError"], false, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn error(v: Value) -> Value {
    assert_eq!(v["isError"], true, "{v}");
    serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[test]
fn only_3d_schemas_change_from_the_frozen_catalog() {
    let original: Value =
        serde_json::from_str(include_str!("../fixtures/catalog-v1.2.0.json")).unwrap();
    let source: Value =
        serde_json::from_str(include_str!("../fixtures/scene3d-source-catalog.json")).unwrap();
    assert_eq!(catalog::tools().len(), 67);
    for tool in catalog::tools() {
        let name = tool["name"].as_str().unwrap();
        if ["edit_3d", "get_3d_scene"].contains(&name) {
            assert_eq!(
                tool,
                source["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["name"] == name)
                    .unwrap()
            );
        } else {
            assert_eq!(
                tool,
                original["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["name"] == name)
                    .unwrap()
            );
        }
    }
}
#[test]
fn create_read_material_and_ordinary_animation_are_drivable_by_id() {
    let f = Fixture::new();
    let mut s = f.session();
    let r=ok(s.call("edit_3d",json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"roundedBox"}}})));
    assert_eq!(r["result"]["id"], "model");
    ok(s.call("edit_3d",json!({"operation":"apply_material","target":"model","preset":"glass","p":{"dispersion":0.2}})));
    ok(s.call("apply_commands",json!({"commands":[{"type":"set_property","target":"model","path":"m.clearcoat","value":0.5}]})));
    let read = ok(s.call("get_3d_scene", json!({"target":"model"})));
    assert_eq!(
        read["scene"]["objects"][0]["material"]["p"]["transmission"]["v"],
        1
    );
    assert_eq!(
        read["scene"]["objects"][0]["material"]["p"]["clearcoat"]["v"],
        0.5
    );
    assert!(
        read["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["layerId"] == "model" && c["path"] == "m.dispersion")
    );
}
#[test]
fn invalid_edit_rolls_back_every_channel() {
    let f = Fixture::new();
    let mut s = f.session();
    ok(s.call("edit_3d", json!({"operation":"add_camera","id":"camera"})));
    let before = f.read();
    error(s.call("edit_3d",json!({"operation":"set_camera","target":"camera","patch":{"p":{"x":30,"near":500,"far":1}}})));
    assert_eq!(f.read(), before);
}
#[test]
fn studio_geometry_ops_and_rollback_are_real_kernel_edits() {
    let f = Fixture::new();
    let before = f.read();
    let mut s = f.session();
    ok(s.call("edit_3d", json!({"operation":"add_object","id":"model"})));
    ok(s.call(
        "edit_3d",
        json!({"operation":"drop_to_floor","target":"model"}),
    ));
    ok(s.call("edit_3d", json!({"operation":"auto_studio"})));
    assert_eq!(f.read()["layers"].as_array().unwrap().len(), 5);
    ok(s.call("edit_3d", json!({"operation":"shot","shot":"push_in"})));
    ok(s.call("rollback_changes", json!({})));
    let mut after = f.read();
    let mut expected = before;
    after.as_object_mut().unwrap().remove("revision");
    expected.as_object_mut().unwrap().remove("revision");
    assert_eq!(after, expected);
}
#[test]
fn stale_session_cannot_apply_3d_commands() {
    let f = Fixture::new();
    let mut first = f.session();
    let mut second = f.session();
    ok(first.call("edit_3d", json!({"operation":"add_object","id":"model"})));
    let e = error(second.call("edit_3d", json!({"operation":"add_light","id":"light"})));
    assert_eq!(e["code"], "revision_conflict");
    assert_eq!(f.read()["layers"].as_array().unwrap().len(), 1);
}
#[test]
fn headless_light_and_camera_content_is_inspectable() {
    let f = Fixture::new();
    let mut s = f.session();
    ok(s.call(
        "edit_3d",
        json!({"operation":"add_light","id":"key","light":{"type":"area"}}),
    ));
    ok(s.call("edit_3d",json!({"operation":"add_camera","id":"camera","camera":{"camera":{"p":{"fStop":2.8,"focusDistance":5}},"environment":{"samples":256}}})));
    let read = ok(s.call("get_3d_scene", json!({})));
    assert_eq!(read["layers"][0]["name"], "Camera");
    assert_eq!(read["layers"][1]["name"], "Softbox");
    assert_eq!(read["scene"]["lights"][0]["p"]["width"]["v"], 2);
    assert_eq!(read["scene"]["camera"]["p"]["fStop"]["v"], 2.8);
    assert_eq!(read["scene"]["environment"]["samples"], 256);
}
#[test]
fn unloaded_imported_geometry_is_a_visible_failure() {
    let f = Fixture::new();
    let mut s = f.session();
    ok(s.call(
        "edit_3d",
        json!({"operation":"add_object","id":"model","object":{"source":{"assetId":"offline"}}}),
    ));
    let before = f.read();
    let e = error(s.call(
        "edit_3d",
        json!({"operation":"drop_to_floor","target":"model"}),
    ));
    assert!(e["message"].as_str().unwrap().contains("not loaded"));
    assert_eq!(f.read(), before);
}
