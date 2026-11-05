use pm_scene3d::{
    migration,
    operations::{self, Action},
    schema::{self, Role},
    service,
};
use serde_json::{Value, json};
fn comp() -> Value {
    json!({"id":"C","w":640,"h":360,"fps":30,"dur":10,"layers":[]})
}
fn plan(args: Value) -> pm_scene3d::Result<operations::Plan> {
    let mut n = 0;
    operations::plan(&comp(), &args, &[], &mut |p| {
        n += 1;
        format!("{p}{n}")
    })
}
#[test]
fn schema_defaults_are_idempotent() {
    let scene = schema::parse_scene(json!({})).unwrap();
    assert_eq!(schema::parse_scene(scene.clone()).unwrap(), scene);
    assert_eq!(scene["environment"]["samples"], 64);
    assert_eq!(scene["camera"]["p"]["fStop"]["v"], 0.);
}
#[test]
fn every_studio_material_field_roundtrips() {
    let p = json!({"clearcoat":1,"clearcoatRoughness":0.002,"transmission":1,"ior":2.6,"specular":0.4,"sheen":1,"iridescence":1200,"dispersion":1,"droplets":1,"anisotropy":-1,"subsurface":1000});
    let m = schema::material(json!({"p":p,"screen":"C2"})).unwrap();
    for (k, v) in p.as_object().unwrap() {
        assert_eq!(m["p"][k]["v"], *v);
    }
    assert_eq!(m["screen"], "C2");
}
#[test]
fn preserves_authored_key_metadata() {
    let p = json!({"v":0.5,"kf":[{"t":1,"v":0.7,"i":"key1","ei":[-0.2,-0.3],"eo":[0.2,0.3],"hold":true,"future":{"keep":true}}],"expr":"value"});
    assert_eq!(
        schema::validate_channel("material", "roughness", p.clone()).unwrap(),
        p
    );
}
#[test]
fn rejects_graph_cycles_missing_parents_and_reserved_ids() {
    for objects in [
        json!([{"id":"a","source":{"primitive":"box"},"parent":"b"},{"id":"b","source":{"primitive":"box"},"parent":"a"}]),
        json!([{"id":"a","source":{"primitive":"box"},"parent":"missing"}]),
        json!([{"id":"camera","source":{"primitive":"box"}}]),
    ] {
        assert!(schema::parse_scene(json!({"objects":objects})).is_err());
    }
}
#[test]
fn rejects_malformed_sources() {
    for source in [
        json!({"primitive":"box","parameters":{"segments":2}}),
        json!({"lathe":[[-1,0],[1,1]]}),
        json!({"extrude":[[0,0],[1,0]],"depth":1}),
        json!({"mesh":{"positions":[0,0,0,1,0,0,0,1,0],"indices":[0,1,3]}}),
        json!({"mesh":{"positions":[0,0,0,1,0,0,0,1,0],"normals":[0,0,1]}}),
    ] {
        assert!(schema::source(&source).is_err(), "{source}");
    }
}
#[test]
fn rejects_unknowns_and_unsafe_edits() {
    for v in [
        json!({"operation":"create","__proto__":{}}),
        json!({"operation":"create","future":true}),
        json!({"operation":"add_object","object":{"source":{"primitive":"cube"}}}),
    ] {
        assert!(plan(v).is_err());
    }
}
#[test]
fn material_map_slots_replace_not_merge() {
    let l=service::new_layer(&comp(),"model","Model",Role::Object,json!({"object":{"source":{"primitive":"box"},"material":{"maps":{"color":"a","normal":"b"}}}}),json!({}),true,None,0.).unwrap();
    let mut c = comp();
    c["layers"] = json!([l]);
    let p=operations::plan(&c,&json!({"operation":"update_object","target":"model","patch":{"material":{"maps":{"roughness":"c"}}}}),&[],&mut|_|"id".into()).unwrap();
    let Action::Data { data, .. } = &p.actions[0] else {
        panic!()
    };
    assert_eq!(data["object"]["material"]["maps"], json!({"roughness":"c"}));
}
#[test]
fn enforces_mesh_budget_and_camera_planes() {
    let object =
        json!({"id":"a","source":{"primitive":"roundedBox","parameters":{"segments":128}}});
    assert!(
        schema::parse_scene(
            json!({"objects":[object.clone(),{"id":"b", "source":object["source"]}]})
        )
        .is_err()
    );
    assert!(schema::camera(json!({"p":{"near":10,"far":1}})).is_err());
}
#[test]
fn source_defaults_light_names_and_visibility() {
    for (kind, name) in [
        ("area", "Softbox"),
        ("spot", "Spotlight"),
        ("point", "Bulb"),
        ("sun", "Sun"),
    ] {
        let p = plan(json!({"operation":"add_light","light":{"type":kind}})).unwrap();
        let Action::Add { name: actual, .. } = &p.actions[0] else {
            panic!()
        };
        assert_eq!(actual, name);
    }
    assert!(plan(json!({"operation":"create","object":{"source":{"primitive":"box"},"p":{"visible":{"v":true,"kf":[{"t":0,"v":false}]}}}})).is_err());
}
#[test]
fn topology_is_parent_first() {
    let p=plan(json!({"operation":"create","scene":{"objects":[{"id":"child","parent":"root","source":{"primitive":"sphere"}},{"id":"root","source":{"primitive":"box"}}]}})).unwrap();
    let Action::Add {
        id: root,
        parent: None,
        ..
    } = &p.actions[0]
    else {
        panic!()
    };
    let Action::Add {
        parent: Some(p), ..
    } = &p.actions[1]
    else {
        panic!()
    };
    assert_eq!(p, root);
}
#[test]
fn layer_data_separates_native_transform() {
    for role in [Role::Object, Role::Light, Role::Camera] {
        let data = match role {
            Role::Object => json!({"object":{"source":{"primitive":"box"}}}),
            Role::Light => json!({"light":{}}),
            Role::Camera => json!({"camera":{}}),
        };
        let d = schema::layer_data(role, data).unwrap();
        assert!(d[role.key()].get("id").is_none());
        if role != Role::Object {
            assert!(d[role.key()]["p"].get("x").is_none());
        }
        assert_eq!(schema::layer_data(role, d.clone()).unwrap(), d);
    }
}
#[test]
fn assembly_refuses_animated_fallback() {
    let mut c = comp();
    let mut l = service::new_layer(
        &c,
        "m",
        "Model",
        Role::Object,
        json!({"object":{"source":{"primitive":"box"}}}),
        json!({}),
        true,
        None,
        0.,
    )
    .unwrap();
    l["p"]["position.x"]["expr"] = json!("time");
    c["layers"] = json!([l]);
    assert!(service::composition(&c, 1., &mut service::StaticEvaluator).is_err());
}
#[test]
fn static_assembly_keeps_new_fields_and_default_light() {
    let mut c = comp();
    let l=service::new_layer(&c,"m","Model",Role::Object,json!({"object":{"source":{"primitive":"roundedBox"},"material":{"screen":"C2","p":{"transmission":1}}}}),json!({}),true,None,0.).unwrap();
    c["layers"] = json!([l]);
    let s = service::composition(&c, 1., &mut service::StaticEvaluator).unwrap();
    assert_eq!(s["objects"][0]["material"]["screen"], "C2");
    assert_eq!(s["objects"][0]["material"]["p"]["transmission"]["v"], 1);
    assert_eq!(s["lights"][0]["type"], "sun");
}
#[test]
fn legacy_container_archive_and_outer_intent_survive() {
    let mut c = comp();
    let old = json!({"id":"old","name":"Old","type":"extension","from":2,"dur":5,"p":{"position.x":{"v":14,"kf":[],"expr":null}},"fx":[{"id":"fx"}],"masks":[],"lock":true,"future":{"keep":true},"d":{"definition":schema::SCENE,"data":{"scene":{"objects":[{"id":"a","source":{"primitive":"box"}}]}}}});
    c["layers"] = json!([old.clone()]);
    let r = migration::upgrade_project(&mut c).unwrap();
    assert_eq!(r.upgraded, 1);
    assert!(r.issues.is_empty());
    assert_eq!(c["layers"][0]["id"], "old");
    assert_eq!(c["layers"][0]["type"], "group");
    assert_eq!(c["layers"][0]["native3dArchive"]["source"], old);
    let saved = c.clone();
    assert_eq!(migration::upgrade_project(&mut c).unwrap().upgraded, 0);
    assert_eq!(c, saved);
}
#[test]
fn legacy_failure_is_visible_and_lossless() {
    let mut c = comp();
    let old = json!({"id":"old","dur":100,"d":{"definition":"powermove.3d.obj-model","params":{"autoRotate":{"v":true}},"data":{"assetId":"source"}}});
    c["layers"] = json!([old.clone()]);
    let r = migration::upgrade_project(&mut c).unwrap();
    assert_eq!(r.issues.len(), 1);
    assert_eq!(c["layers"][0]["d"]["data"], old["d"]["data"]);
    assert!(c["layers"][0]["native3dUpgradeIssue"].is_string());
}
#[test]
fn every_material_preset_is_valid() {
    let presets = service::presets();
    assert_eq!(presets.as_array().unwrap().len(), 11);
    for p in presets.as_array().unwrap() {
        schema::material(json!({"p":p["p"]})).unwrap();
    }
}

#[test]
fn standalone_upgrade_archives_original_three_d_flag() {
    let data = pm_scene3d::schema::layer_data(
        pm_scene3d::schema::Role::Object,
        json!({"object":{"source":{"primitive":"box"}}}),
    )
    .unwrap();
    let original = json!({"id":"object","type":"extension","threeD":false,"d":{"definition":"powermove.3d.object","data":data}});
    let mut project = json!({"layers":[original.clone()]});
    pm_scene3d::migration::upgrade_project(&mut project).unwrap();
    assert_eq!(project["layers"][0]["threeD"], true);
    assert_eq!(project["layers"][0]["native3dArchive"]["source"], original);
}
