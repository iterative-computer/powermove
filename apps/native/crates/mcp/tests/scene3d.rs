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
fn error(s: &mut McpSession, args: Value) -> Value {
    let error = s.dispatch("edit_3d", &args).unwrap_err();
    assert_eq!(
        pm_mcp::failure(error.clone())["content"][0]["text"],
        error.message
    );
    serde_json::to_value(error).unwrap()
}
#[test]
fn only_studio_export_and_project_recovery_schemas_change_from_the_frozen_catalog() {
    let original: Value =
        serde_json::from_str(include_str!("../fixtures/catalog-main-c7f46a056.json")).unwrap();
    let source: Value =
        serde_json::from_str(include_str!("../fixtures/studio3d-native-catalog.json")).unwrap();
    let projects: Value =
        serde_json::from_str(include_str!("../fixtures/project-native-catalog.json")).unwrap();
    assert_eq!(catalog::tools().len(), 68);
    for tool in catalog::tools() {
        let name = tool["name"].as_str().unwrap();
        if ["edit_3d", "get_3d_scene", "export_video"].contains(&name) {
            assert_eq!(
                tool,
                source["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["name"] == name)
                    .unwrap()
            );
        } else if name == "open_project" {
            assert_eq!(tool, &projects["tools"][0]);
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
fn export_quality_is_a_separate_validated_job_option() {
    let args = json!({"studioQuality":{"preset":"production","maxSamples":512}});
    catalog::validate("export_video", &args).unwrap();
    assert_eq!(
        pm_mcp::services::VideoOptions::parse(&args)
            .unwrap()
            .studio_quality,
        Some(args["studioQuality"].clone())
    );
    for quality in [
        json!("unknown"),
        json!({"maxSamples":4097}),
        json!({"denoise":7}),
        json!({"unknown":true}),
        json!({"noiseThreshold":-0.1}),
    ] {
        assert!(catalog::validate("export_video", &json!({"studioQuality":quality})).is_err());
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
    error(
        &mut s,
        json!({"operation":"set_camera","target":"camera","patch":{"p":{"x":30,"near":500,"far":1}}}),
    );
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
    let e = error(&mut second, json!({"operation":"add_light","id":"light"}));
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
    let e = error(
        &mut s,
        json!({"operation":"drop_to_floor","target":"model"}),
    );
    assert!(e["message"].as_str().unwrap().contains("not loaded"));
    assert_eq!(f.read(), before);
}

struct GeometryEngine { path: PathBuf }
impl pm_mcp::EngineHook for GeometryEngine {
    fn capture(&self, _: &pm_mcp::CaptureRequest) -> pm_mcp::Result<Value> {
        Err(pm_mcp::Error::unavailable("no renderer needed for geometry commands"))
    }
    fn scene3d_assets(&self, _: &pm_mcp::services::FrozenProject) -> pm_mcp::Result<Arc<dyn pm_scene3d::geometry::AssetGeometry>> {
        Ok(Arc::new(pm_scene3d::geometry::FileAssets::new([("asset".into(), self.path.clone())].into())))
    }
}
#[test]
fn headless_auto_studio_and_floor_tools_load_imported_geometry() {
    let f = Fixture::new();
    let path = f.root.join("original.bin");
    std::fs::write(&path, pm_scene3d::import::triangle_glb(&[[0.,0.,0.],[2.,0.,0.],[0.,4.,0.]], &[0,1,2])).unwrap();
    {
        let mut host = f.host.lock().unwrap();
        let revision = host.revision(&f.id).unwrap();
        host.apply_commands(&f.id, vec![pm_kernel::MediaCommand::AddAsset {
            asset_id:"asset".into(), asset:pm_scene3d::to_model(&json!({"id":"asset","kind":"model","name":"triangle.glb","format":"glb"})),
        }.into()], "Synthetic model", pm_kernel::Origin::Mcp, revision, 1).unwrap();
        host.engine = Some(Arc::new(GeometryEngine { path }));
    }
    let mut session = f.session();
    ok(session.call("edit_3d", json!({"operation":"add_object","id":"model","object":{"source":{"assetId":"asset"}}})));
    ok(session.call("edit_3d", json!({"operation":"auto_studio","target":"model"})));
    ok(session.call("edit_3d", json!({"operation":"drop_to_floor","target":"model"})));
    let scene = ok(session.call("get_3d_scene", json!({})));
    assert!(scene["scene"]["objects"][0]["p"]["y"]["v"].as_f64().unwrap() > 0.);
    assert_eq!(scene["scene"]["lights"].as_array().unwrap().len(), 3);
}

#[test]
fn environment_and_quality_ops_reach_the_kernel_and_readback() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    ok(session.call(
        "edit_3d",
        json!({"operation":"set_world","id":"world","world":{"p":{"elevation":8}}}),
    ));
    ok(session.call(
        "edit_3d",
        json!({"operation":"add_fog","id":"mist","fog":{"p":{"density":0.03}}}),
    ));
    ok(session.call(
        "edit_3d",
        json!({"operation":"set_render_quality","preset":"preview"}),
    ));
    let result = ok(session.call("get_3d_scene", json!({})));
    assert_eq!(result["scene"]["studioQuality"]["preset"], "preview");
    assert!(
        result["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["path"] == "world.elevation" && c["value"] == 8)
    );
    assert!(
        result["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["path"] == "fog.density")
    );
    let before = fixture.read();
    assert!(
        session.call(
            "edit_3d",
            json!({"operation":"set_world","patch":{"source":{"layerId":"world"}}})
        )["isError"]
            == true
    );
    assert_eq!(fixture.read(), before);
}
#[test]
fn sparse_material_commands_and_channels_are_agent_reachable() {
    let fixture = Fixture::new();
    let mut s = fixture.session();
    ok(s.call(
        "edit_3d",
        json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"box"}}}),
    ));
    ok(s.call("edit_3d", json!({"operation":"update_object","target":"model","patch":{
        "materialProperties":{"opacity":75,"roughness":0.6,"emissionIntensity":2,"ambientResponse":25},
        "sourceMaterialProperties":{"1":{"metallic":0.3,"emissionColor":"#00FF00"}},
        "sourceMaterials":{"1":{"color":"paint"}}
    }})));
    ok(s.call("apply_commands", json!({"commands":[{"type":"set_property","target":"model","path":"mi.1.metallic","value":0.8}]})));
    let read = ok(s.call("get_3d_scene", json!({"target":"model"})));
    assert_eq!(
        read["scene"]["objects"][0]["sourceMaterials"]["1"]["color"],
        "paint"
    );
    for (path, value) in [
        ("mi.opacity", json!(75)),
        ("mi.ambientResponse", json!(25)),
        ("mi.1.metallic", json!(0.8)),
    ] {
        assert!(
            read["channels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["path"] == path && c["value"] == value),
            "{path}: {read}"
        );
    }
}

#[test]
fn extrusion_commands_and_surface_channels_are_exposed_to_agents(){
    let f=Fixture::new();let mut s=f.session();
    ok(s.call("apply_commands",json!({"commands":[{"type":"add_layer","id":"title","layerType":"text","content":{"text":"Studio","size":60}}]})));
    ok(s.call("edit_3d",json!({"operation":"set_extrusion","target":"title","patch":{"enabled":true,"depth":25,"bevelStyle":"convex"}})));
    ok(s.call("edit_3d",json!({"operation":"set_surface_material_properties","target":"title","surface":"bevel","patch":{"metallic":0.8,"color":"#FFFFFF"}})));
    ok(s.call("edit_3d",json!({"operation":"set_surface_material_map","target":"title","materialIndex":2,"slot":"color","map":"paint"})));
    ok(s.call("apply_commands",json!({"commands":[{"type":"set_property","target":"title","path":"ms.bevel.metallic","value":0.6}]})));
    let read=ok(s.call("get_3d_scene",json!({"target":"title"})));
    assert_eq!(read["layers"][0]["role"],"extrusion");
    assert_eq!(read["scene"]["extrusions"][0]["extrusion"]["p"]["depth"]["v"],25);
    assert_eq!(read["scene"]["extrusions"][0]["extrusion"]["surfaces"]["side"]["maps"]["color"],"paint");
    assert!(read["channels"].as_array().unwrap().iter().any(|c|c["path"]=="ms.bevel.metallic" && c["value"]==0.6));
}

#[test]
fn video_render_scale_is_explicit_validated_and_defaults_to_full_size() {
    use pm_mcp::{catalog,services::VideoOptions};
    let args=serde_json::json!({"studioRenderScale":0.75});
    catalog::validate("export_video",&args).unwrap();
    assert_eq!(VideoOptions::parse(&args).unwrap().studio_render_scale,Some(0.75));
    assert_eq!(VideoOptions::parse(&serde_json::json!({})).unwrap().studio_render_scale,None);
    for value in [serde_json::json!(0),serde_json::json!(-1),serde_json::json!(1.1),serde_json::json!(true)] {
        let args=serde_json::json!({"studioRenderScale":value});
        assert!(catalog::validate("export_video",&args).is_err());
        assert!(VideoOptions::parse(&args).is_err());
    }
}
