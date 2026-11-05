mod common;
use pm_kernel::commands::{layers::LayerPatch, properties::SetProperty};
use pm_kernel::{Command, DocumentStore, LayerCommand, Origin, PropertyCommand, Scene3dCommand};
use pm_scene3d::{from_model, to_model};
use serde_json::{Value, json};
fn store() -> DocumentStore {
    DocumentStore::new_untitled(to_model(
        &json!({"id":"P","name":"3D","compId":"C","w":640,"h":360,"fps":30,"dur":10,"layers":[],"assets":{}}),
    ))
}
fn command(args: Value) -> Command {
    Scene3dCommand::Apply {
        comp: None,
        args: to_model(&args),
        selection: vec![],
        time: 0.,
        autokey: false,
    }
    .into()
}
fn apply(s: &mut DocumentStore, args: Value) {
    s.execute(command(args), Origin::Interface).unwrap();
}
fn project(s: &DocumentStore) -> Value {
    from_model(&s.snapshot().root().unwrap().to_value()).unwrap()
}
fn layer(s: &DocumentStore, id: &str) -> Value {
    let p = project(s);
    p["comps"]["C"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap()
        .clone()
}
#[test]
fn create_update_undo_redo_is_exact() {
    let mut s = store();
    let before = project(&s);
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"box"},"p":{"sx":2}}}),
    );
    assert_eq!(layer(&s, "model")["p"]["scale.x"]["v"], 200.);
    let created = project(&s);
    s.undo().unwrap();
    assert_eq!(project(&s), before);
    s.redo().unwrap();
    assert_eq!(project(&s), created);
    apply(
        &mut s,
        json!({"operation":"update_object","target":"model","patch":{"material":{"p":{"transmission":1},"maps":{"color":"texture"}}}}),
    );
    let l = layer(&s, "model");
    assert_eq!(
        l["d"]["data"]["object"]["material"]["p"]["transmission"]["v"],
        1
    );
    assert_eq!(
        l["d"]["data"]["object"]["material"]["maps"]["color"],
        "texture"
    );
    s.undo().unwrap();
    assert_eq!(project(&s), created);
}
#[test]
fn failure_is_atomic_and_preserves_history() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_camera","id":"cam"}));
    let p = project(&s);
    let revision = s.revision();
    assert!(
        s.execute(
            command(
                json!({"operation":"set_camera","target":"cam","patch":{"p":{"near":2000,"far":1}}})
            ),
            Origin::Mcp
        )
        .is_err()
    );
    assert_eq!(s.revision(), revision);
    assert_eq!(project(&s), p);
}
#[test]
fn coupled_camera_pair_is_validated_at_end() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_camera","id":"cam"}));
    apply(
        &mut s,
        json!({"operation":"set_camera","target":"cam","patch":{"p":{"near":2000,"far":3000}}}),
    );
    assert_eq!(
        layer(&s, "cam")["d"]["data"]["camera"]["p"]["near"]["v"],
        2000
    );
}
#[test]
fn ordinary_property_commands_find_3d_channels() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_object","id":"model"}));
    s.execute(
        PropertyCommand::Set(SetProperty::new("model", "m.clearcoat", 0.7, 0.)).into(),
        Origin::Interface,
    )
    .unwrap();
    assert_eq!(
        layer(&s, "model")["d"]["data"]["object"]["material"]["p"]["clearcoat"]["v"],
        0.7
    );
    let before = project(&s);
    assert!(
        s.execute(
            PropertyCommand::Set(SetProperty::new("model", "m.clearcoat", 2., 0.)).into(),
            Origin::Mcp
        )
        .is_err()
    );
    assert_eq!(project(&s), before);
}
#[test]
fn locks_and_hand_edits_are_enforced() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_object","id":"model"}));
    let mut property = SetProperty::new("model", "m.roughness", 0.6, 0.);
    property.mark_intent = true;
    s.execute(PropertyCommand::Set(property).into(), Origin::Interface)
        .unwrap();
    assert!(s.execute(command(json!({"operation":"update_object","target":"model","patch":{"material":{"p":{"roughness":0.4}}}})),Origin::Mcp).is_err());
    s.execute(
        LayerCommand::Set {
            layer_id: "model".into(),
            patch: LayerPatch {
                locked: Some(true),
                ..Default::default()
            },
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
    assert!(
        s.execute(
            command(json!({"operation":"remove","target":"model"})),
            Origin::Mcp
        )
        .is_err()
    );
}
#[test]
fn native_channels_preserve_temporal_metadata_on_full_assignment() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_object","id":"model"}));
    let c = json!({"v":0.5,"kf":[{"t":0,"v":0.3,"i":"authored","ei":[-0.2,0.1],"eo":[0.1,0.2],"hold":true,"future":"keep"}],"expr":null});
    apply(
        &mut s,
        json!({"operation":"update_object","target":"model","patch":{"material":{"p":{"roughness":c}}}}),
    );
    assert_eq!(
        layer(&s, "model")["d"]["data"]["object"]["material"]["p"]["roughness"],
        c
    );
}
#[test]
fn shots_are_normal_tracks_and_one_undo() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_camera","id":"cam"}));
    let p = project(&s);
    apply(
        &mut s,
        json!({"operation":"shot","shot":"orbit","duration":4}),
    );
    assert_eq!(
        layer(&s, "cam")["p"]["position.x"]["kf"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
    s.undo().unwrap();
    assert_eq!(project(&s), p);
}
#[test]
fn duplicate_remaps_all_3d_key_ids() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"sphere"},"material":{"p":{"roughness":{"v":0.4,"kf":[{"t":0,"v":0.5,"i":"key"}]}}}}}),
    );
    apply(
        &mut s,
        json!({"operation":"duplicate","target":"model","name":"Copy"}),
    );
    let p = project(&s);
    let ls = p["comps"]["C"]["layers"].as_array().unwrap();
    let copy = ls.iter().find(|l| l["name"] == "Copy").unwrap();
    assert_ne!(
        copy["d"]["data"]["object"]["material"]["p"]["roughness"]["kf"][0]["i"],
        "key"
    );
    assert_eq!(
        layer(&s, "model")["d"]["data"]["object"]["material"]["p"]["roughness"]["kf"][0]["i"],
        "key"
    );
}
#[test]
fn saved_pmv_reopens_exact_content_and_undo() {
    let dir = common::TempDir::new("scene3d");
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"roundedBox"},"material":{"screen":"screen","p":{"iridescence":700}}}}),
    );
    let p = project(&s);
    let path = dir.path("scene.pmv");
    common::save_as(&mut s, &path).unwrap();
    let mut reopened = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(project(&reopened), p);
    reopened.undo().unwrap();
    assert!(
        project(&reopened)["comps"]["C"]["layers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
