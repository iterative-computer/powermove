use pm_mcp::{Automation, McpSession};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Fixture {
    // Release the session and its actor before removing the isolated profile.
    session: McpSession,
    host: Arc<Mutex<Automation>>,
    id: String,
    _scratch: Scratch,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("mcp-captions"));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let mut host = Automation::new(root.join("profile"), root.join("exports")).unwrap();
        let created=host.create_project(&json!({"name":"Synthetic captions","width":640,"height":360,"fps":25,"duration":20})).unwrap();
        let id = created["projectId"].as_str().unwrap().to_owned();
        let host = Arc::new(Mutex::new(host));
        let mut session = McpSession::new(host.clone());
        session
            .dispatch("start_session", &json!({"prompt":"Synthetic captions"}))
            .unwrap();
        Self {
            session,
            host,
            id,
            _scratch: Scratch(root),
        }
    }
    fn apply(&mut self, commands: Value) -> Value {
        self.session
            .dispatch("apply_commands", &json!({"commands":commands}))
            .unwrap()
    }
    fn layer(&self) -> Value {
        self.host.lock().unwrap().read_document(&self.id).unwrap()["layers"][0].clone()
    }
}
#[test]
fn caption_batch_uses_the_same_ids_for_planning_actor_and_later_commands() {
    let mut f = Fixture::new();
    let response=f.apply(json!([
  {"type":"add_captions","id":"caption","from":3,"duration":4,"cues":[{"start":4,"end":6,"text":"First words"},{"start":6,"end":7,"text":"Second"}],"language":"en"},
  {"type":"edit_captions","target":"caption","op":"style","style":{"size":52},"language":"fr"}
 ]));
    let layer = f.layer();
    assert_eq!(layer["d"]["cues"][0]["start"], 1.);
    assert_eq!(layer["d"]["language"], "fr");
    assert_eq!(response["results"][0]["data"]["id"], "caption");
    let cue = layer["d"]["cues"][0]["id"].as_str().unwrap();
    let response =
        f.apply(json!([{"type":"edit_captions","target":"caption","op":"split","id":cue,"at":5.}]));
    let tail = response["results"][0]["data"]["tailId"].clone();
    assert!(
        f.layer()["d"]["cues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cue| cue["id"] == tail)
    );
    f.apply(json!([{"type":"edit_captions","target":"caption","op":"delete","ids":[tail]}]));
    assert_eq!(f.layer()["d"]["cues"].as_array().unwrap().len(), 2);
    f.session.dispatch("rollback_changes", &json!({})).unwrap();
    assert!(
        f.host.lock().unwrap().read_document(&f.id).unwrap()["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn text_import_insert_and_explicit_empty_move_preserve_stable_cue_identity() {
    let mut f = Fixture::new();
    f.apply(json!([{"type":"add_captions","id":"caption","text":"1\n00:00:04,000 --> 00:00:05,000\nFirst\n","format":"srt","from":3}]));
    let before = f.layer()["d"]["cues"].clone();
    f.apply(json!([{"type":"edit_captions","target":"caption","op":"move","ids":[],"by":1.}]));
    assert_eq!(f.layer()["d"]["cues"], before);
    f.apply(
        json!([{"type":"edit_captions","target":"caption","op":"move","by":1.,"language":"fr"}]),
    );
    let layer = f.layer();
    assert_eq!(layer["d"]["cues"][0]["start"], 2.);
    assert_eq!(layer["d"]["language"], "fr");
    f.apply(json!([{"type":"edit_captions","target":"caption","op":"import","text":"1\n00:00:08,000 --> 00:00:09,000\nSecond\n","format":"srt","replace":false}]));
    let layer = f.layer();
    assert_eq!(layer["d"]["cues"].as_array().unwrap().len(), 2);
    assert_ne!(layer["d"]["cues"][0]["id"], layer["d"]["cues"][1]["id"]);
}

#[test]
fn generic_add_caption_reports_the_same_normalized_cue_identity_the_actor_admits() {
    let mut f = Fixture::new();
    let response=f.apply(json!([{"type":"add_layer","id":"caption","layerType":"captions","content":{"cues":[{"start":1,"end":2,"text":"Synthetic"}]}}]));
    let projected = &response["results"][0]["data"]["layer"];
    let actual = f.layer();
    assert_eq!(actual["p"]["opacity"]["v"], 100.);
    assert_eq!(actual["d"]["cues"], projected["d"]["cues"]);
    let cue = actual["d"]["cues"][0]["id"].clone();
    f.apply(json!([{"type":"edit_captions","target":"caption","op":"update","cues":[{"id":cue,"text":"Updated"}]}]));
    assert_eq!(f.layer()["d"]["cues"][0]["text"], "Updated");
}
