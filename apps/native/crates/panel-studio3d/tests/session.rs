use pm_kernel::{
    Call, Caller, Command, KernelActor, KernelConfig, OpenSource, Reply, Request, Scene3dCommand,
    SelectionCall, WriteBody,
};
use pm_panel_studio3d::Session;
use pm_scene3d::{from_model, to_model};
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};
struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
fn run<F: Future>(future: F) -> F::Output {
    let w = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&w);
    let mut f = std::pin::pin!(future);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}
fn tokens() -> pm_design::Tokens {
    pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default())
}
fn fixture() -> (KernelActor, Session) {
    let a = KernelActor::spawn(KernelConfig::default());
    a.handle().open("p",OpenSource::Untitled(to_model(&json!({"id":"p","compId":"c","w":640,"h":360,"fps":30,"dur":10,"layers":[],"assets":{},"comps":{}})))).wait().unwrap().unwrap();
    let s = Session::new(a.handle(), "p", Caller::native());
    (a, s)
}
fn publish(s: &mut Session) -> Value {
    s.publish(0., 900., false, false, &tokens()).unwrap()
}
fn drive(s: &mut Session, id: &str, v: Value) {
    let state = publish(s);
    run(s.drive(state["ticket"].as_str().unwrap(), id, v)).unwrap();
}
fn call(a: &KernelActor, c: impl Into<Call>) {
    let ack = a
        .handle()
        .call("p", Request::new(Caller::native(), c))
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
}
fn select(a: &KernelActor, id: &str) {
    call(
        a,
        SelectionCall::Layers {
            ids: vec![id.into()],
            add: false,
        },
    );
}
fn root(a: &KernelActor) -> Value {
    from_model(
        &a.handle()
            .published("p")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
    )
    .unwrap()
}
fn created(a: &KernelActor) -> String {
    root(a)["comps"]["c"]["layers"]
        .as_array()
        .unwrap()
        .first()
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .into()
}
fn edit(a: &KernelActor, args: Value) {
    let c: Command = Scene3dCommand::Apply {
        comp: Some("c".into()),
        args: to_model(&args),
        selection: vec![],
        time: 0.,
        autokey: false,
    }
    .into();
    call(
        a,
        Call::Transact {
            label: None,
            body: WriteBody::Commands(vec![c]),
            expected_revision: None,
        },
    );
}
#[test]
fn toolbar_uses_actual_tokens_and_source_suppression() {
    let (_a, mut s) = fixture();
    assert_eq!(publish(&mut s)["visible"], false);
    drive(&mut s, "add", json!("box"));
    let t = tokens();
    for (width, overlay, layout) in [
        (t.metrics.viewer_3d_labels, false, "labels"),
        (t.metrics.viewer_3d_labels - 1., false, "icons"),
        (t.metrics.viewer_3d_collapse - 1., false, "menu"),
        (1000., true, "icons"),
    ] {
        let v = s.publish(0., width, false, overlay, &t).unwrap();
        assert_eq!(v["layout"], layout);
        assert_eq!(v["visible"], true);
    }
    assert_eq!(
        s.publish(0., 900., true, false, &t).unwrap()["visible"],
        false
    );
}
#[test]
fn all_add_menu_items_reach_real_kernel() {
    let (a, mut s) = fixture();
    for kind in [
        "box",
        "sphere",
        "plane",
        "cylinder",
        "cone",
        "torus",
        "capsule",
        "icosahedron",
        "area",
        "spot",
        "point",
        "sun",
        "camera",
    ] {
        drive(&mut s, "add", json!(kind));
    }
    let p = root(&a);
    assert_eq!(p["comps"]["c"]["layers"].as_array().unwrap().len(), 13);
    assert_eq!(a.handle().published("p").unwrap().history.entries, 13);
}
#[test]
fn inspector_edits_and_undo_are_real_and_exact() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    let id = created(&a);
    select(&a, &id);
    let before = root(&a);
    drive(&mut s, "field.m.roughness", json!(0.7));
    let i = publish(&mut s);
    assert_eq!(
        i["inspector"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == "m.roughness")
            .unwrap()["value"],
        0.7
    );
    call(&a, pm_kernel::HistoryCall::Undo);
    assert_eq!(root(&a), before);
}
#[test]
fn rejects_stale_publications_and_unknown_agent_only_controls() {
    let (a, mut s) = fixture();
    let old = publish(&mut s);
    drive(&mut s, "add", json!("box"));
    assert!(run(s.drive(old["ticket"].as_str().unwrap(), "add", json!("camera"))).is_err());
    select(&a, &created(&a));
    for id in [
        "explode",
        "place_on",
        "soften",
        "compatibility",
        "rendered",
        "tool.unknown",
    ] {
        let v = publish(&mut s);
        assert!(run(s.drive(v["ticket"].as_str().unwrap(), id, Value::Null)).is_err());
    }
}
#[test]
fn tools_preserve_transform_under_navigation_and_never_edit_camera() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    select(&a, &created(&a));
    let before = root(&a);
    drive(&mut s, "tool.rotate", Value::Null);
    drive(&mut s, "tool.orbit", Value::Null);
    let v = publish(&mut s);
    assert_eq!(v["tools"]["transform"], "rotate");
    assert_eq!(v["tools"]["navigation"], "orbit");
    drive(&mut s, "tool.light", Value::Null);
    drive(&mut s, "tool.move", Value::Null);
    assert!(!s.tools.light);
    assert_eq!(root(&a), before);
}
#[test]
fn restrictions_cover_camera_sun_and_bulb() {
    let (a, mut s) = fixture();
    for (kind, disabled) in [
        ("camera", vec!["scale"]),
        ("sun", vec!["move", "scale"]),
        ("point", vec!["rotate", "scale"]),
    ] {
        drive(&mut s, "add", json!(kind));
        select(&a, &created(&a));
        let v = publish(&mut s);
        for tool in disabled {
            assert_eq!(v["allowed"][tool], false);
            assert!(
                run(s.drive(
                    v["ticket"].as_str().unwrap(),
                    &format!("tool.{tool}"),
                    Value::Null
                ))
                .is_err()
            );
        }
    }
}
#[test]
fn quality_preference_survives_orthographic_fallback() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("camera"));
    select(&a, &created(&a));
    drive(&mut s, "preview", json!("quality"));
    drive(&mut s, "projection", json!("orthographic"));
    let v = publish(&mut s);
    assert_eq!(v["quality"], false);
    assert_eq!(v["qualityStored"], true);
    assert!(run(s.drive(v["ticket"].as_str().unwrap(), "preview", json!("quality"))).is_err());
    drive(&mut s, "projection", json!("perspective"));
    assert_eq!(publish(&mut s)["quality"], true);
}
#[test]
fn gestures_commit_one_undo_and_escape_restores_exactly() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    select(&a, &created(&a));
    let before = root(&a);
    let count = a.handle().published("p").unwrap().history.entries;
    let v = publish(&mut s);
    run(s.begin(v["ticket"].as_str().unwrap(), "Change Roughness")).unwrap();
    for n in [0.3, 0.4, 0.8] {
        run(s.step("field.m.roughness", json!(n))).unwrap();
    }
    run(s.end(false)).unwrap();
    assert_eq!(
        a.handle().published("p").unwrap().history.entries,
        count + 1
    );
    call(&a, pm_kernel::HistoryCall::Undo);
    assert_eq!(root(&a), before);
    let v = publish(&mut s);
    run(s.begin(v["ticket"].as_str().unwrap(), "Change Roughness")).unwrap();
    run(s.step("field.m.roughness", json!(0.2))).unwrap();
    run(s.dispose()).unwrap();
    assert_eq!(root(&a), before);
}
#[test]
fn keyframe_diamond_reads_evaluated_channel_and_toggles() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    let id = created(&a);
    select(&a, &id);
    drive(&mut s, "key.m.roughness", Value::Null);
    let v = publish(&mut s);
    let f = v["inspector"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "m.roughness")
        .unwrap();
    assert_eq!(f["animated"], true);
    assert_eq!(f["keyed"], true);
    drive(&mut s, "key.m.roughness", Value::Null);
    assert_eq!(publish(&mut s)["inspector"]["fields"][1]["animated"], false);
    edit(
        &a,
        json!({"operation":"update_object","target":id,"patch":{"material":{"p":{"roughness":{"v":0.1,"kf":[{"t":0,"v":0.2},{"t":2,"v":0.8}],"expr":null}}}}}),
    );
    let v = s.publish(1., 900., false, false, &tokens()).unwrap();
    let f = v["inspector"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "m.roughness")
        .unwrap();
    assert_eq!(f["value"], 0.5);
    assert_eq!(f["keyed"], false);
}
#[test]
fn source_materials_hide_override_fields_and_finishes_use_source_values() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    select(&a, &created(&a));
    drive(&mut s, "finish", json!("gold"));
    let v = publish(&mut s);
    let preset = pm_scene3d::service::presets();
    let gold = preset
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "gold")
        .unwrap();
    assert_eq!(
        v["inspector"]["data"]["object"]["material"]["p"]["color"]["v"],
        gold["p"]["color"]
    );
    drive(&mut s, "useSourceMaterials", json!(true));
    assert!(
        publish(&mut s)["inspector"]["fields"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn shots_and_final_quality_are_kernel_tracks_and_settings() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("camera"));
    select(&a, &created(&a));
    for shot in ["push_in", "orbit", "hero_tilt", "rack_focus"] {
        let before = root(&a);
        drive(&mut s, "shot", json!(shot));
        assert_ne!(root(&a), before);
        call(&a, pm_kernel::HistoryCall::Undo);
        assert_eq!(root(&a), before);
    }
    drive(&mut s, "samples", json!(256));
    assert_eq!(
        publish(&mut s)["inspector"]["data"]["environment"]["samples"],
        256
    );
}
#[test]
fn primitive_geometry_setup_and_drop_use_real_evaluator() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    select(&a, &created(&a));
    for op in ["drop_to_floor", "auto_studio"] {
        let v = publish(&mut s);
        run(s.geometry(
            v["ticket"].as_str().unwrap(),
            op,
            &pm_scene3d::geometry::NoAssets,
        ))
        .unwrap();
    }
    assert_eq!(
        root(&a)["comps"]["c"]["layers"].as_array().unwrap().len(),
        5
    );
}
#[test]
fn multiple_selection_hides_properties_and_bad_values_are_atomic() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    let x = created(&a);
    drive(&mut s, "add", json!("sphere"));
    let y = created(&a);
    call(
        &a,
        SelectionCall::Layers {
            ids: vec![x.clone(), y],
            add: false,
        },
    );
    assert!(publish(&mut s)["inspector"].is_null());
    select(&a, &x);
    let before = root(&a);
    for arg in [json!(-1), json!(2), json!("oops")] {
        let v = publish(&mut s);
        assert!(run(s.drive(v["ticket"].as_str().unwrap(), "field.m.roughness", arg)).is_err());
        assert_eq!(root(&a), before);
    }
}
#[test]
fn light_adjustments_use_real_channels_and_preserve_unrelated_animation() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("area"));
    let id = created(&a);
    select(&a, &id);
    edit(
        &a,
        json!({"operation":"update_light","target":id,"patch":{"p":{"width":{"v":2,"kf":[{"t":0,"v":2,"i":"width-key"},{"t":2,"v":4,"i":"width-key2"}],"expr":null}}}}),
    );
    let before = root(&a);
    drive(&mut s, "tool.light", Value::Null);
    let v = publish(&mut s);
    assert_eq!(v["lightPanel"]["rows"].as_array().unwrap().len(), 3);
    drive(&mut s, "light.adjust.brightness", json!(7.5));
    let after = root(&a);
    let old = &before["comps"]["c"]["layers"][0]["d"]["data"]["light"]["p"];
    let now = &after["comps"]["c"]["layers"][0]["d"]["data"]["light"]["p"];
    assert_eq!(now["width"], old["width"]);
    assert_eq!(now["intensity"]["v"], 7.5);
    drive(&mut s, "light.kind", json!("spot"));
    let v = publish(&mut s);
    assert_eq!(
        v["lightPanel"]["hint"],
        "Click a model to add a Spotlight there."
    );
}
#[test]
fn frame_dispatch_changes_only_free_view_with_real_geometry() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    select(&a, &created(&a));
    let before = root(&a);
    let count = a.handle().published("p").unwrap().history.entries;
    let v = publish(&mut s);
    run(s.dispatch(
        json!({"ticket":v["ticket"],"element":"frame","argument":null}),
        &pm_scene3d::geometry::NoAssets,
    ))
    .unwrap();
    let v = publish(&mut s);
    assert_eq!(v["tools"]["free"], true);
    assert!(v["freePose"]["position"].is_array());
    assert_eq!(root(&a), before);
    assert_eq!(a.handle().published("p").unwrap().history.entries, count);
}
#[test]
fn changed_gesture_selection_refuses_instead_of_editing_a_new_target() {
    let (a, mut s) = fixture();
    drive(&mut s, "add", json!("box"));
    let x = created(&a);
    drive(&mut s, "add", json!("sphere"));
    let y = created(&a);
    select(&a, &x);
    let v = publish(&mut s);
    run(s.begin(v["ticket"].as_str().unwrap(), "Change Roughness")).unwrap();
    select(&a, &y);
    let before = root(&a);
    assert!(run(s.step("field.m.roughness", json!(0.9))).is_err());
    run(s.end(true)).unwrap();
    assert_eq!(root(&a), before);
}
#[test]
fn durable_import_is_one_undo_and_originals_survive_undo() {
    use pm_scene3d::import::{self, ImportFile, archive::Archive};
    let (a, mut s) = fixture();
    let before = root(&a);
    let temp = std::env::temp_dir().join(format!(
        "pm-studio-ui-import-{}-{}",
        std::process::id(),
        a.handle().published("p").unwrap().snapshot.doc_id
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let temp = std::fs::canonicalize(temp).unwrap();
    let archive = Archive::open(&temp.join("archive")).unwrap();
    let bytes = import::triangle_glb(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[0, 1, 2]);
    let receipt = Arc::new(
        archive
            .admit(
                import::prepare(
                    vec![ImportFile {
                        name: "Product.glb".into(),
                        mime: "model/gltf-binary".into(),
                        bytes: bytes.clone(),
                    }],
                    None,
                )
                .unwrap(),
            )
            .unwrap(),
    );
    let v = publish(&mut s);
    run(s.import(
        v["ticket"].as_str().unwrap(),
        "asset".into(),
        "product".into(),
        receipt.clone(),
    ))
    .unwrap();
    assert_eq!(
        root(&a)["comps"]["c"]["layers"].as_array().unwrap().len(),
        5
    );
    assert_eq!(a.handle().published("p").unwrap().history.entries, 1);
    call(&a, pm_kernel::HistoryCall::Undo);
    assert_eq!(root(&a), before);
    assert_eq!(std::fs::read(receipt.primary().path()).unwrap(), bytes);
    std::fs::remove_dir_all(temp).unwrap();
}
#[test]
fn automatic_upgrade_reaches_panel_without_mutating_archived_source() {
    let a = KernelActor::spawn(KernelConfig::default());
    let old = json!({"id":"old","name":"Old","type":"extension","from":0,"dur":5,"lock":true,"p":{},"d":{"definition":"powermove.3d.scene","data":{"scene":{"objects":[{"id":"product","source":{"primitive":"box"}}]}}}});
    a.handle().open("p",OpenSource::Untitled(to_model(&json!({"id":"p","compId":"c","w":640,"h":360,"fps":30,"dur":10,"layers":[old.clone()],"assets":{}})))).wait().unwrap().unwrap();
    let mut s = Session::new(a.handle(), "p", Caller::native());
    let p = root(&a);
    let ls = p["comps"]["c"]["layers"].as_array().unwrap();
    let group = ls.iter().find(|l| l["id"] == "old").unwrap();
    assert_eq!(group["native3dArchive"]["source"], old);
    assert_eq!(group["lock"], true);
    let model = ls
        .iter()
        .find(|l| l["d"]["definition"] == "powermove.3d.object")
        .unwrap();
    select(&a, model["id"].as_str().unwrap());
    let v = publish(&mut s);
    assert_eq!(v["visible"], true);
    assert_eq!(v["inspector"]["role"], "object");
    assert_eq!(a.handle().published("p").unwrap().history.entries, 0);
}
#[test]
fn screen_cycles_are_disabled_and_refused_and_none_clears_reference() {
    let a = KernelActor::spawn(KernelConfig::default());
    let model = pm_scene3d::service::new_layer(
        &json!({"w":640,"h":360,"fps":30,"dur":10}),
        "product",
        "Product",
        pm_scene3d::schema::Role::Object,
        json!({"object":{"source":{"primitive":"box"}}}),
        json!({}),
        true,
        None,
        0.,
    )
    .unwrap();
    a.handle().open("p",OpenSource::Untitled(to_model(&json!({"id":"p","compId":"c","assets":{},"comps":{
        "c":{"id":"c","name":"Shot","w":640,"h":360,"fps":30,"dur":10,"layers":[model]},
        "screen":{"id":"screen","name":"Screen","w":640,"h":360,"fps":30,"dur":10,"layers":[]},
        "cycle":{"id":"cycle","name":"Cycle","w":640,"h":360,"fps":30,"dur":10,"layers":[{"id":"nested","type":"comp","comp":"c","from":0,"dur":10,"p":{}}]}
    }})))).wait().unwrap().unwrap();
    select(&a, "product");
    let mut s = Session::new(a.handle(), "p", Caller::native());
    let v = publish(&mut s);
    for id in ["c", "cycle"] {
        assert_eq!(
            v["inspector"]["screens"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["v"] == id)
                .unwrap()["disabled"],
            true
        );
        assert!(run(s.drive(v["ticket"].as_str().unwrap(), "screen", json!(id))).is_err());
    }
    drive(&mut s, "screen", json!("screen"));
    assert_eq!(
        publish(&mut s)["inspector"]["data"]["object"]["material"]["screen"],
        "screen"
    );
    drive(&mut s, "screen", Value::Null);
    assert!(publish(&mut s)["inspector"]["data"]["object"]["material"]["screen"].is_null());
}
#[test]
fn disclosures_persist_in_kernel_ui_without_touching_project_or_unknown_fields() {
    let (a, mut s) = fixture();
    let before = root(&a);
    drive(&mut s, "disclosure.Surface", json!(true));
    let v = publish(&mut s);
    assert_eq!(v["disclosures"]["Surface"], true);
    assert_eq!(root(&a), before);
    assert_eq!(a.handle().published("p").unwrap().history.entries, 0);
}
