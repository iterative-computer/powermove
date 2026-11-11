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
fn sparse_material_commands_channels_history_and_pmv_roundtrip() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"box"}}}),
    );
    let before = project(&s);
    s.execute(command(json!({"operation":"update_object","target":"model","patch":{
        "materialProperties":{"opacity":75,"roughness":0.6,"emissionIntensity":2,"ambientResponse":25},
        "sourceMaterialProperties":{"1":{"metallic":0.3,"emissionColor":"#00FF00"}},
        "sourceMaterials":{"1":{"color":"paint","emissive":"paint"}}
    }})), Origin::Mcp).unwrap();
    let authored = project(&s);
    s.undo().unwrap();
    assert_eq!(project(&s), before);
    s.redo().unwrap();
    assert_eq!(project(&s), authored);
    s.execute(
        PropertyCommand::Set(SetProperty::new("model", "mi.1.metallic", 0.8, 0.)).into(),
        Origin::Interface,
    )
    .unwrap();
    let resolved = pm_kernel::commands::scene3d::resolve_scene(&s.snapshot(), None, 0.).unwrap();
    assert_eq!(
        resolved.scene["objects"][0]["sourceMaterialProperties"]["1"]["metallic"]["v"],
        0.8
    );
    let saved = project(&s);
    let mut dir = common::TempDir::new("sparse-material");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let path = dir.path("scene.pmv");
    common::save_as(&mut s, &path).unwrap();
    let mut reopened = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(project(&reopened), saved);
    reopened.undo().unwrap();
    assert_eq!(project(&reopened), authored);
    reopened.redo().unwrap();
    assert_eq!(project(&reopened), saved);
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
    let mut dir = common::TempDir::new("scene3d");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
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
#[test]
fn animated_legacy_upgrade_uses_original_expression_context() {
    let p = json!({"id":"P","name":"Legacy","compId":"C","w":640,"h":360,"fps":30,"dur":2,"layers":[{"id":"old","name":"Old","type":"extension","from":0,"dur":1,"p":{},"d":{"definition":"powermove.3d.obj-model","data":{"assetId":"asset"},"params":{"yaw":{"v":0,"kf":[],"expr":"time * 90"},"autoRotate":{"v":true,"kf":[],"expr":null},"speed":{"v":25,"kf":[],"expr":null}}}}],"assets":{}});
    let s = DocumentStore::new_untitled(to_model(&p));
    let root = project(&s);
    let layers = root["comps"]["C"]["layers"].as_array().unwrap();
    assert_eq!(layers[0]["type"], "group");
    let camera = layers
        .iter()
        .find(|l| l["d"]["definition"] == "powermove.3d.camera")
        .unwrap();
    assert_eq!(
        camera["p"]["position.x"]["kf"].as_array().unwrap().len(),
        31
    );
    let model = layers
        .iter()
        .find(|l| l["d"]["definition"] == "powermove.3d.object")
        .unwrap();
    assert_eq!(model["p"]["rotation.y"]["kf"].as_array().unwrap().len(), 31);
    assert_eq!(model["p"]["rotation.y"]["kf"][30]["v"], 50.);
}
#[test]
fn legacy_history_and_redo_are_upgraded_and_original_archived() {
    let mut dir = common::TempDir::new("scene3d-history");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let p = json!({"id":"P","name":"History","compId":"C","w":640,"h":360,"fps":30,"dur":10,"layers":[{"id":"old","name":"Edited","type":"extension","from":0,"dur":5,"p":{},"d":{"definition":"powermove.3d.scene","data":{"scene":{"objects":[{"id":"box","source":{"primitive":"box"}}]}}}}],"assets":{}});
    let path = json!(["comps", "C", "layers", 0, "name"]);
    let history = json!({"version":1,"paths":"composition","index":0,"entries":[{"label":"Rename","forward":[{"path":path,"exists":true,"value":"Edited"}],"backward":[{"path":path,"exists":true,"value":"Original"}]},{"label":"Rename again","forward":[{"path":path,"exists":true,"value":"Redo"}],"backward":[{"path":path,"exists":true,"value":"Edited"}]}]});
    let file = dir.path("legacy.json");
    std::fs::write(&file, json!({"proj":p,"history":history}).to_string()).unwrap();
    let mut s = DocumentStore::open(
        pm_kernel::project_io::open_project(&file, Default::default()).unwrap(),
    );
    assert_eq!(layer(&s, "old")["type"], "group");
    assert_eq!(layer(&s, "old")["name"], "Edited");
    assert_eq!(project(&s)["native3dHistoryArchive"], history);
    s.undo().unwrap();
    assert_eq!(layer(&s, "old")["name"], "Original");
    assert_eq!(layer(&s, "old")["type"], "group");
    s.redo().unwrap();
    s.redo().unwrap();
    assert_eq!(layer(&s, "old")["name"], "Redo");
    let output = dir.path("native.pmv");
    common::save_as(&mut s, &output).unwrap();
    let p = project(&s);
    let mut s = DocumentStore::open(
        pm_kernel::project_io::open_project(&output, Default::default()).unwrap(),
    );
    assert_eq!(project(&s), p);
    s.undo().unwrap();
    assert_eq!(layer(&s, "old")["name"], "Edited");
}
fn geometric(s: &mut DocumentStore, args: Value) {
    let geometry = pm_kernel::commands::scene3d::prepare_geometry(
        &s.snapshot(),
        None,
        &to_model(&args),
        &[],
        0.,
        &pm_scene3d::geometry::NoAssets,
    )
    .unwrap();
    s.execute(
        Scene3dCommand::ApplyPrepared {
            comp: None,
            args: to_model(&args),
            selection: vec![],
            time: 0.,
            autokey: false,
            geometry,
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
}
#[test]
fn drop_to_floor_uses_real_bounds_and_rejects_stale_preflight() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"box"},"p":{"y":5}}}),
    );
    let args = json!({"operation":"drop_to_floor","target":"model"});
    let geometry = pm_kernel::commands::scene3d::prepare_geometry(
        &s.snapshot(),
        None,
        &to_model(&args),
        &[],
        0.,
        &pm_scene3d::geometry::NoAssets,
    )
    .unwrap();
    apply(
        &mut s,
        json!({"operation":"update_object","target":"model","patch":{"name":"Changed"}}),
    );
    let before = project(&s);
    assert!(
        s.execute(
            Scene3dCommand::ApplyPrepared {
                comp: None,
                args: to_model(&args),
                selection: vec![],
                time: 0.,
                autokey: false,
                geometry
            }
            .into(),
            Origin::Mcp
        )
        .is_err()
    );
    assert_eq!(project(&s), before);
    geometric(&mut s, args);
    assert_eq!(layer(&s, "model")["p"]["position.y"]["v"], 0.5);
}
#[test]
fn auto_studio_is_one_atomic_edit() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_object","id":"model"}));
    let before = project(&s);
    geometric(&mut s, json!({"operation":"auto_studio"}));
    let p = project(&s);
    let ls = p["comps"]["C"]["layers"].as_array().unwrap();
    assert_eq!(ls.len(), 5);
    assert_eq!(
        ls.iter()
            .filter(|l| l["d"]["definition"] == "powermove.3d.light"
                && l["d"]["data"]["light"]["type"] == "area")
            .count(),
        3
    );
    s.undo().unwrap();
    assert_eq!(project(&s), before);
}
#[test]
fn place_on_uses_world_bounds() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_object","id":"stand","object":{"source":{"primitive":"box"},"p":{"x":4,"y":1,"sy":2}}}),
    );
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"sphere"},"p":{"x":-2,"y":5}}}),
    );
    geometric(
        &mut s,
        json!({"operation":"place_on","target":"model","on_target":"stand"}),
    );
    assert_eq!(layer(&s, "model")["p"]["position.x"]["v"], 4.);
    let args = to_model(&json!({"operation":"place_on","target":"model","on_target":"stand"}));
    let g = pm_kernel::commands::scene3d::prepare_geometry(
        &s.snapshot(),
        None,
        &args,
        &[],
        0.,
        &pm_scene3d::geometry::NoAssets,
    )
    .unwrap();
    assert!(
        (g.bounds["model"][1].as_f64().unwrap() - g.bounds["stand"][4].as_f64().unwrap()).abs()
            < 1e-10
    );
}
#[test]
fn exploded_group_writes_normal_position_tracks() {
    let mut s = store();
    for (id, x) in [("left", -2), ("right", 2)] {
        apply(
            &mut s,
            json!({"operation":"add_object","id":id,"object":{"source":{"primitive":"box"},"p":{"x":x}}}),
        );
    }
    // A saved native model group. The general GroupCommand's geometry-host
    // integration is outside this test's scene-command contract.
    let mut p = project(&s);
    let ls = p["comps"]["C"]["layers"].as_array_mut().unwrap();
    for l in ls.iter_mut() {
        l["group"] = json!("group");
    }
    ls.insert(0,json!({"id":"group","type":"group","name":"Group","threeD":true,"on":true,"from":0,"dur":10,"p":{},"d":{}}));
    let mut s = DocumentStore::new_untitled(to_model(&p));
    let before = project(&s);
    geometric(
        &mut s,
        json!({"operation":"explode","target":"group","duration":2}),
    );
    assert_eq!(
        layer(&s, "left")["p"]["position.x"]["kf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        layer(&s, "right")["p"]["position.x"]["kf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    s.undo().unwrap();
    assert_eq!(project(&s), before);
}
#[test]
fn imported_glb_and_originals_survive_undo_reopen_and_archive_recovery() {
    use pm_scene3d::import::{self, ImportFile, archive::Archive};
    let mut dir = common::TempDir::new("scene3d-import");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let bytes = import::triangle_glb(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[0, 1, 2]);
    let archive = Archive::open(&dir.path("archive")).unwrap();
    let receipt = std::sync::Arc::new(
        archive
            .admit(
                import::prepare(
                    vec![ImportFile {
                        name: "product.glb".into(),
                        mime: "model/gltf-binary".into(),
                        bytes: bytes.clone(),
                    }],
                    None,
                )
                .unwrap(),
            )
            .unwrap(),
    );
    let mut s = store();
    let before = project(&s);
    s.execute(
        Scene3dCommand::Import {
            comp: None,
            asset_id: "asset".into(),
            layer_id: "model".into(),
            receipt: receipt.clone(),
            time: 0.,
            setup_studio: true,
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
    assert_eq!(
        project(&s)["comps"]["C"]["layers"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(s.media().len(), 1);
    let imported = project(&s);
    s.undo().unwrap();
    assert_eq!(project(&s), before);
    assert!(receipt.primary().path().exists());
    s.redo().unwrap();
    assert_eq!(project(&s), imported);
    let path = dir.path("import.pmv");
    common::save_as(&mut s, &path).unwrap();
    let mut reopened = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(reopened.media().len(), 1);
    assert_eq!(
        imported["assets"]["asset"]["native3dSources"][0]["id"],
        "asset"
    );
    let out = dir.path("asset.bin");
    reopened
        .media()
        .get("asset")
        .unwrap()
        .copy_to(&out)
        .unwrap();
    assert_eq!(std::fs::read(out).unwrap(), bytes);
    reopened.undo().unwrap();
    assert!(project(&reopened)["assets"].get("asset").is_none());
    // Simulate journal/library recovery with metadata and the durable content
    // archive, without relying on a PMV media range or an original source file.
    let mut recovered = DocumentStore::new_untitled(to_model(&imported));
    let recovered_receipt =
        std::sync::Arc::new(archive.recover(&imported["assets"]["asset"]).unwrap());
    let rev = recovered.revision();
    recovered
        .execute(
            Scene3dCommand::BindImport {
                asset_id: "asset".into(),
                receipt: recovered_receipt,
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
    assert_eq!(recovered.revision(), rev);
    assert_eq!(recovered.media().len(), 1);
    common::save_as(&mut recovered, &dir.path("recovered.pmv")).unwrap();
}
#[test]
fn normal_add_layer_and_content_commands_support_registered_3d_definitions() {
    use pm_kernel::commands::layers::AddLayer;
    let mut s = store();
    let content=to_model(&json!({"definition":"powermove.3d.object","data":{"object":{"source":{"primitive":"box"}}}})).as_object().unwrap().clone();
    s.execute(
        LayerCommand::Add(Box::new(AddLayer {
            id: "model".into(),
            layer_type: "extension".into(),
            comp: None,
            name: None,
            from: None,
            duration: None,
            content,
            properties: to_model(&json!({"opacity":40}))
                .as_object()
                .unwrap()
                .clone(),
            color: None,
            index: None,
            parent: None,
            blend: None,
            motion_blur: None,
            visible: None,
            shy: None,
            collapsed: None,
            time: 0.,
        }))
        .into(),
        Origin::Extension,
    )
    .unwrap();
    assert_eq!(layer(&s, "model")["p"]["opacity"]["v"], 40);
    let patch = to_model(&json!({"data":{"object":{"source":{"primitive":"sphere"}}}}))
        .as_object()
        .unwrap()
        .clone();
    s.execute(
        pm_kernel::ContentCommand::Set {
            layer_id: "model".into(),
            patch,
            time: 0.,
        }
        .into(),
        Origin::Extension,
    )
    .unwrap();
    assert_eq!(
        layer(&s, "model")["d"]["data"]["object"]["source"]["primitive"],
        "sphere"
    );
}
#[test]
fn full_channel_assignment_validates_expression_and_keeps_keys_addressable() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"box"},"p":{"sx":{"v":1,"kf":[{"t":0,"v":1}]},"sy":{"v":1,"kf":[{"t":0,"v":1}]}}}}),
    );
    let l = layer(&s, "model");
    let a = &l["p"]["scale.x"]["kf"][0]["i"];
    let b = &l["p"]["scale.y"]["kf"][0]["i"];
    assert!(a.is_string() && b.is_string());
    assert_ne!(a, b);
    let before = project(&s);
    assert!(s.execute(command(json!({"operation":"update_object","target":"model","patch":{"material":{"p":{"roughness":{"v":0.5,"kf":[],"expr":"("}}}}})),Origin::Interface).is_err());
    assert_eq!(project(&s), before);
}
#[test]
fn resolved_service_uses_the_native_world_and_opacity_evaluator() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"box"},"p":{"x":3,"y":4,"z":5}}}),
    );
    s.execute(
        PropertyCommand::Set(SetProperty::new("model", "opacity", 40., 0.)).into(),
        Origin::Interface,
    )
    .unwrap();
    apply(
        &mut s,
        json!({"operation":"add_light","id":"light","light":{"type":"area"}}),
    );
    apply(&mut s, json!({"operation":"add_camera","id":"camera"}));
    let resolved = pm_kernel::commands::scene3d::resolve_scene(&s.snapshot(), None, 0.).unwrap();
    assert_eq!(&resolved.model_worlds["model"][12..15], &[3., 4., 5.]);
    assert_eq!(resolved.opacities["model"], 0.4);
    assert_eq!(resolved.camera_id.as_deref(), Some("camera"));
    assert_eq!(resolved.targets["light"], [0.; 3]);
}

#[test]
fn unrepresentable_unicode_keeps_original_units_and_reports_failed_upgrade() {
    let input = pm_model::json::parse(r#"{"id":"P","compId":"C","w":8,"h":8,"fps":30,"dur":1,"unknown":"\ud800","layers":[{"id":"legacy","ty":"extension","d":{"definition":"powermove.3d.obj-model","data":{"assetId":"a"}}}],"assets":{}}"#).unwrap();
    let original = input.get("layers").unwrap().as_array().unwrap()[0].clone();
    let mut s = DocumentStore::new_untitled(input.clone());
    let root = s.snapshot().root().unwrap().to_value();
    assert_eq!(root.get("unknown"), input.get("unknown"));
    let l = &root
        .get("comps")
        .unwrap()
        .get("C")
        .unwrap()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()[0];
    assert_eq!(
        l.get("native3dArchive").unwrap().get("source"),
        Some(&original)
    );
    assert_eq!(
        l.get("d").unwrap().get("definition").unwrap().as_str(),
        Some("powermove.3d.unupgraded")
    );
    assert!(l.get("native3dUpgradeIssue").is_some());
    let mut dir = common::TempDir::new("scene3d-unicode");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let path = dir.path("retained.pmv");
    common::save_as(&mut s, &path).unwrap();
    let reopened = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(
        reopened.snapshot().root().unwrap().to_value(),
        s.snapshot().root().unwrap().to_value()
    );
}

#[test]
fn splat_import_undo_reopen_preserves_exact_source() {
    use pm_scene3d::import::{self, ImportFile, archive::Archive};
    let mut dir = common::TempDir::new("scene3d-splat-save");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let mut bytes = [0f32, 0., 0., 1., 1., 1.]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    bytes.extend([255, 128, 32, 255, 255, 128, 128, 128]);
    let receipt = std::sync::Arc::new(
        Archive::open(&dir.path("archive"))
            .unwrap()
            .admit(
                import::prepare(
                    vec![ImportFile {
                        name: "scene.splat".into(),
                        mime: "".into(),
                        bytes: bytes.clone(),
                    }],
                    None,
                )
                .unwrap(),
            )
            .unwrap(),
    );
    let mut s = store();
    apply(&mut s, json!({"operation":"add_camera","id":"cam"}));
    s.execute(
        Scene3dCommand::Import {
            comp: None,
            asset_id: "splat".into(),
            layer_id: "model".into(),
            receipt,
            time: 0.,
            setup_studio: true,
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
    let imported = project(&s);
    let path = dir.path("splat.pmv");
    common::save_as(&mut s, &path).unwrap();
    let mut reopened = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(project(&reopened), imported);
    let out = dir.path("source.splat");
    reopened
        .media()
        .get("splat")
        .unwrap()
        .copy_to(&out)
        .unwrap();
    assert_eq!(std::fs::read(out).unwrap(), bytes);
    reopened.undo().unwrap();
    assert!(project(&reopened)["assets"].get("splat").is_none());
}

#[test]
fn studio_environment_quality_channels_undo_and_pmv_are_exact() {
    let mut s = store();
    let edits = [
        json!({"operation":"set_world","id":"world","world":{"mode":"sky","preset":"golden-hour","p":{"elevation":8,"fogDensity":0.003}}}),
        json!({"operation":"add_fog","id":"mist","fog":{"p":{"density":0.035}}}),
        json!({"operation":"add_terrain","id":"ground","terrain":{"p":{"resolution":8,"seed":41},"regions":[{"p":{"minSlope":32,"color":"#8C7D64"}}]}}),
        json!({"operation":"add_scatter","id":"trees","scatter":{"model":"nature:conifer","surface":"ground","p":{"count":12,"seed":71}}}),
        json!({"operation":"update_fog","target":"mist","patch":{"p":{"density":{"v":0.02,"kf":[{"t":2,"v":0.05,"i":"fog-key","future":"keep"}],"expr":null}}}}),
        json!({"operation":"set_material_options","target":"ground","patch":{"castsShadows":"only","appearsInReflections":false}}),
        json!({"operation":"set_render_quality","preset":"preview"}),
        json!({"operation":"set_render_quality","patch":{"maxSamples":128}}),
    ];
    for edit in edits {
        let before = project(&s);
        apply(&mut s, edit);
        let after = project(&s);
        s.undo().unwrap();
        assert_eq!(project(&s), before);
        s.redo().unwrap();
        assert_eq!(project(&s), after);
    }
    s.execute(
        PropertyCommand::Set(SetProperty::new("world", "world.azimuth", 30., 0.)).into(),
        Origin::Interface,
    )
    .unwrap();
    s.execute(
        PropertyCommand::Set(SetProperty::new(
            "ground",
            "terrain.region0.roughness",
            0.7,
            0.,
        ))
        .into(),
        Origin::Interface,
    )
    .unwrap();
    assert_eq!(
        layer(&s, "world")["d"]["data"]["object"]["world"]["p"]["azimuth"]["v"],
        30.
    );
    assert_eq!(
        layer(&s, "ground")["d"]["data"]["object"]["terrain"]["regions"][0]["p"]["roughness"]["v"],
        0.7
    );
    let saved = project(&s);
    let mut dir = common::TempDir::new("studio-contract");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let path = dir.path("scene.pmv");
    common::save_as(&mut s, &path).unwrap();
    let reopened = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(project(&reopened), saved);
    assert_eq!(saved["comps"]["C"]["studioQuality"]["preset"], "custom");
}

#[test]
fn environment_cycles_ranges_and_locks_fail_atomically() {
    let mut s = store();
    apply(&mut s, json!({"operation":"set_world","id":"world"}));
    apply(
        &mut s,
        json!({"operation":"add_terrain","id":"ground","terrain":{"p":{"resolution":8},"regions":[{"p":{}}]}}),
    );
    for edit in [
        json!({"operation":"set_world","patch":{"source":{"layerId":"world"}}}),
        json!({"operation":"add_scatter","scatter":{"model":"ground","surface":"ground","p":{"scaleMin":2,"scaleMax":1}}}),
        json!({"operation":"update_terrain","target":"ground","patch":{"heights":[0,1]}}),
        json!({"operation":"set_render_quality","patch":{"minSamples":4000,"maxSamples":32}}),
    ] {
        let before = project(&s);
        assert!(s.execute(command(edit), Origin::Mcp).is_err());
        assert_eq!(project(&s), before);
    }
    let mut property = SetProperty::new("world", "world.intensity", 2., 0.);
    property.mark_intent = true;
    s.execute(PropertyCommand::Set(property).into(), Origin::Interface)
        .unwrap();
    let before = project(&s);
    assert!(
        s.execute(
            command(json!({"operation":"set_world","patch":{"p":{"intensity":3}}})),
            Origin::Mcp
        )
        .is_err()
    );
    assert_eq!(project(&s), before);
}

#[test]
fn structural_region_replacement_preserves_hand_edited_channels() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"add_terrain","id":"ground","terrain":{"regions":[{"p":{}}]}}),
    );
    let mut p = SetProperty::new("ground", "terrain.region0.roughness", 0.7, 0.);
    p.mark_intent = true;
    s.execute(PropertyCommand::Set(p).into(), Origin::Interface)
        .unwrap();
    let before = project(&s);
    assert!(s.execute(command(json!({"operation":"update_terrain","target":"ground","patch":{"regions":[{"p":{"roughness":0.4}}]}})),Origin::Mcp).is_err());
    assert_eq!(project(&s), before);
}

#[test]
fn fbx_import_undo_and_pmv_keep_converted_and_original_bytes() {
    use pm_scene3d::import::{self, ImportFile, archive::Archive};
    let original =
        include_bytes!("../../../../../engine/crates/pm-import-fbx/fixtures/tiny-animated.fbx");
    let prepared = import::prepare(
        vec![ImportFile {
            name: "animated.fbx".into(),
            mime: "model/fbx".into(),
            bytes: original.to_vec(),
        }],
        None,
    )
    .unwrap();
    let converted = prepared.bytes.clone();
    let mut dir = common::TempDir::new("fbx-roundtrip");
    dir.0 = std::fs::canonicalize(&dir.0).unwrap();
    let archive = Archive::open(&dir.path("archive")).unwrap();
    let receipt = std::sync::Arc::new(archive.admit(prepared).unwrap());
    let mut s = store();
    s.execute(
        Scene3dCommand::Import {
            comp: None,
            asset_id: "fbx".into(),
            layer_id: "product".into(),
            receipt,
            time: 0.,
            setup_studio: false,
        }
        .into(),
        Origin::Interface,
    )
    .unwrap();
    assert_eq!(project(&s)["assets"]["fbx"]["format"], "glb");
    let saved = project(&s);
    let path = dir.path("scene.pmv");
    common::save_as(&mut s, &path).unwrap();
    let mut reopened = DocumentStore::open(
        pm_kernel::project_io::open_project(&path, Default::default()).unwrap(),
    );
    assert_eq!(project(&reopened), saved);
    for (id, expected) in [
        ("fbx", converted.as_slice()),
        ("fbx-source-0", original.as_slice()),
    ] {
        let output = dir.path(&format!("{id}.bin"));
        reopened.media().get(id).unwrap().copy_to(&output).unwrap();
        assert_eq!(std::fs::read(output).unwrap(), expected);
    }
    reopened.undo().unwrap();
    assert!(project(&reopened)["assets"].get("fbx").is_none());
    reopened.redo().unwrap();
    assert_eq!(project(&reopened), saved);
}

#[test]
fn sky_presets_write_source_defaults_and_quality_combines_preset_with_patch() {
    let mut s = store();
    apply(
        &mut s,
        json!({"operation":"set_world","id":"world","world":{"preset":"golden-hour","p":{"haze":5}}}),
    );
    assert_eq!(
        layer(&s, "world")["d"]["data"]["object"]["world"]["p"]["elevation"]["v"],
        6.
    );
    assert_eq!(
        layer(&s, "world")["d"]["data"]["object"]["world"]["p"]["haze"]["v"],
        5.
    );
    let before = project(&s);
    apply(
        &mut s,
        json!({"operation":"set_world","patch":{"preset":"night"}}),
    );
    assert_eq!(
        layer(&s, "world")["d"]["data"]["object"]["world"]["p"]["elevation"]["v"],
        -20.
    );
    s.undo().unwrap();
    assert_eq!(project(&s), before);
    apply(
        &mut s,
        json!({"operation":"set_render_quality","preset":"draft","patch":{"maxSamples":8}}),
    );
    let q = &project(&s)["comps"]["C"]["studioQuality"];
    assert_eq!(q["minSamples"], 4);
    assert_eq!(q["maxSamples"], 8);
    assert_eq!(q["maxBounces"], 4);
}

#[test]
fn legacy_sample_command_updates_composition_quality_in_one_undo_step() {
    let mut s = store();
    apply(&mut s, json!({"operation":"add_camera","id":"cam"}));
    let before = project(&s);
    apply(
        &mut s,
        json!({"operation":"set_environment","patch":{"samples":12}}),
    );
    let after = project(&s);
    let q = &after["comps"]["C"]["studioQuality"];
    assert_eq!(q["maxSamples"], 12);
    assert_eq!(q["minSamples"], 12);
    assert_eq!(q["preset"], "custom");
    assert_eq!(layer(&s, "cam")["d"]["data"]["environment"]["samples"], 12);
    s.undo().unwrap();
    assert_eq!(project(&s), before);
    s.redo().unwrap();
    assert_eq!(project(&s), after);
    let revision = s.revision();
    assert!(
        s.execute(
            command(json!({"operation":"set_environment","patch":{"samples":0}})),
            Origin::Interface
        )
        .is_err()
    );
    assert_eq!(s.revision(), revision);
    assert_eq!(project(&s), after);
}

#[test]
fn extrusion_surfaces_channels_undo_and_pmv_roundtrip() {
    let mut s=DocumentStore::new_untitled(to_model(&json!({"id":"P","compId":"C","w":640,"h":360,"fps":30,"dur":10,"assets":{},"layers":[{"id":"title","name":"Title","type":"text","on":true,"from":0,"dur":10,"p":{},"d":{"text":"Studio","font":"Arial","fontSize":60,"color":"#E8E2CF"}}]})));
    let flat=project(&s);
    apply(&mut s,json!({"operation":"set_extrusion","target":"title","patch":{"enabled":true,"depth":30,"bevelStyle":"convex","bevelDepth":4}}));
    let extruded=project(&s);
    assert_eq!(layer(&s,"title")["threeD"],true);
    assert_eq!(layer(&s,"title")["d"]["extrusion"]["p"]["depth"]["v"],30);
    s.undo().unwrap();assert_eq!(project(&s),flat);s.redo().unwrap();assert_eq!(project(&s),extruded);
    for (i,surface) in pm_scene3d::extrusion::SURFACES.iter().enumerate() {
        apply(&mut s,json!({"operation":"set_surface_material_properties","target":"title","materialIndex":i,"patch":{"color":"#FFFFFF","roughness":0.25,"metallic":0.8}}));
        apply(&mut s,json!({"operation":"set_surface_material_map","target":"title","surface":surface,"slot":"normal","map":"paint"}));
        s.execute(PropertyCommand::Set(SetProperty::new("title",format!("ms.{surface}.roughness"),0.7,0.)).into(),Origin::Mcp).unwrap();
        assert_eq!(layer(&s,"title")["d"]["extrusion"]["surfaces"][surface]["properties"]["roughness"]["v"],0.7);
    }
    let authored=project(&s);
    assert!(s.execute(command(json!({"operation":"set_extrusion","target":"title","patch":{"depth":-1}})),Origin::Mcp).is_err());
    assert!(s.execute(command(json!({"operation":"set_material_map","target":"title","slot":"normal","map":{"layerId":"title"}})),Origin::Mcp).is_err());
    assert_eq!(project(&s),authored);
    let mut dir=common::TempDir::new("extrusion");dir.0=std::fs::canonicalize(&dir.0).unwrap();let path=dir.path("text.pmv");
    common::save_as(&mut s,&path).unwrap();
    let mut reopened=DocumentStore::open(pm_kernel::project_io::open_project(&path,Default::default()).unwrap());
    assert_eq!(project(&reopened),authored);reopened.undo().unwrap();reopened.redo().unwrap();assert_eq!(project(&reopened),authored);
    apply(&mut reopened,json!({"operation":"set_material_properties","target":"title","surface":"back","patch":{"roughness":null}}));
    assert!(layer(&reopened,"title")["d"]["extrusion"]["surfaces"]["back"]["properties"].get("roughness").is_none());
    reopened.undo().unwrap();assert_eq!(project(&reopened),authored);
}
