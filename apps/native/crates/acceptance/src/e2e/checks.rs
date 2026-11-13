use crate::fixture::{Fixture, layer, tool_error, without_revision};
use pm_kernel::{
    Call, Caller, Command, DocumentStore, HistoryCall, KernelActor, KernelConfig, OpenSource,
    Origin, Reply, Request, SelectionCall, WriteBody,
};
use pm_mcp::{ActorAutomation, CaptureRequest, EngineHook, SurfaceHook};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub type Check = Result<(), String>;
pub struct NativeCheck {
    pub id: &'static str,
    pub run: fn() -> Check,
}

// This is the executable registry, shared by cargo test and the JSON report.
macro_rules! registry {
    ($($name:ident),+ $(,)?) => {
        pub const ALL: &[NativeCheck] = &[
            $(NativeCheck { id: stringify!($name), run: $name }),+
        ];
        #[cfg(test)] mod tests {
            $(#[test] fn $name() { super::$name().unwrap(); })+
        }
    }
}
registry!(
    easing_replace_batch,
    revision_recovery,
    selection_preserves_redo,
    strip_move_and_library,
    speed_double,
    speed_half,
    speed_undo_redo,
    video_split_continuity,
    video_slip_bounds,
    video_trim_bounds,
    keyframe_drag_values,
    markers_persist,
    live_drag_commit,
    live_drag_cancel,
    authored_fractional_rates,
    portable_opaque_data,
    portable_redo_tail,
    library_trash_retains_work,
    media_folder_undo,
    media_delete_lock,
    caption_srt,
    caption_vtt,
    capture_preserves_document,
    capture_rejects_stale,
    capture_failure_preserves_edits,
    surface_stable_ids,
    surface_unknown_ids,
    mcp_tool_contract,
    pending_tools_are_explicit,
    easing_set_batch
);

fn start(f: &mut Fixture) -> Check {
    f.call("start_session", json!({"prompt":"Synthetic frozen e2e"}))?;
    Ok(())
}
fn commit(f: &mut Fixture) -> Check {
    f.call("finish_session", json!({"commit":true}))?;
    Ok(())
}
fn undo(f: &Fixture) -> Check {
    f.host
        .lock()
        .unwrap()
        .undo(&f.id, Origin::Mcp, 71)
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn store(f: &mut Fixture) -> Result<DocumentStore, String> {
    let saved = f
        .host
        .lock()
        .unwrap()
        .save_project(&f.id)
        .map_err(|e| e.to_string())?;
    let opened = pm_kernel::project_io::open_project(
        std::path::Path::new(saved["path"].as_str().ok_or("missing path")?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(DocumentStore::open(opened))
}
fn verify_saved(f: &mut Fixture) -> Check {
    let before = without_revision(f.document()?);
    assert_eq!(without_revision(document(&store(f)?)), before);
    Ok(())
}
fn document(store: &DocumentStore) -> Value {
    let value = pm_model::canonical::realize_project(store.snapshot().document.to_value().unwrap());
    serde_json::from_str(&pm_model::json::to_canonical_string(&value)).unwrap()
}
fn evaluate(doc: Value, id: &str, path: &str, time: f64) -> f64 {
    let mut store = DocumentStore::new_untitled(pm_model::json::parse(&doc.to_string()).unwrap());
    store
        .queries()
        .evaluate(None, id, path, time)
        .unwrap()
        .as_f64()
        .unwrap()
}

/// Shared unchanged independent authored-curve oracle.
fn easing(operation: &str) -> Check {
    crate::curve_oracle::easing(operation)
}
fn easing_replace_batch() -> Check {
    easing("replace_keyframes")
}
fn easing_set_batch() -> Check {
    easing("set_property")
}

/// agent-revision-recovery.spec.ts:3: the exact 90-layer / 150-key case.
fn revision_recovery() -> Check {
    let mut f = Fixture::new()?;
    // A different writer advances the project after this session's baseline.
    let batch = pm_mcp::commands::plan(
        &f.document()?,
        &[],
        &[json!({"type":"add_layer","id":"user-layer","layerType":"text"})],
        0.,
    )
    .map_err(|e| e.to_string())?;
    foreign(&f, batch.commands)?;
    let before = f.document()?;
    let commands = Value::Array((0..90).map(|i| json!({
        "type":"add_layer","id":format!("agent-{i}"),"layerType":"text","content":{"text":format!("Title {i}")}
    })).collect());
    let error = tool_error(
        &mut f.session,
        "apply_commands",
        json!({"commands":commands}),
    )?;
    assert_eq!(error["code"], "revision_conflict");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("get_project_state")
    );
    assert_eq!(f.document()?, before);
    f.call("get_project_state", json!({}))?;
    f.edit(commands)?;
    assert_eq!(f.document()?["layers"].as_array().unwrap().len(), 91);
    let user = before["layers"][0]["id"].as_str().unwrap();
    foreign(
        &f,
        vec![Command::SetLayerName {
            layer_id: user.into(),
            name: "User revision".into(),
        }],
    )?;
    f.call("get_project_state", json!({}))?;
    let keys = Value::Array(
        (0..150)
            .map(|i| json!({"time":f64::from(i)/30.,"value":i%100}))
            .collect(),
    );
    f.edit(
        json!([{"type":"replace_keyframes","target":"agent-0","path":"opacity","keyframes":keys}]),
    )?;
    assert_eq!(
        layer(&f.document()?, "agent-0")["p"]["opacity"]["kf"]
            .as_array()
            .unwrap()
            .len(),
        150
    );
    commit(&mut f)?;
    undo(&f)?;
    let after = f.document()?;
    assert_eq!(layer(&after, user)["name"], "User revision");
    assert_eq!(after["layers"].as_array().unwrap().len(), 91);
    assert_eq!(layer(&after, "agent-0")["p"]["opacity"]["kf"], json!([]));
    Ok(())
}
fn foreign(f: &Fixture, commands: Vec<Command>) -> Check {
    let mut h = f.host.lock().unwrap();
    let revision = h.revision(&f.id).map_err(|e| e.to_string())?;
    h.apply_commands(
        &f.id,
        commands,
        "Synthetic foreign writer",
        Origin::Agent,
        revision,
        72,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

struct ActorFixture {
    host: ActorAutomation,
    _actor: KernelActor,
}
impl ActorFixture {
    fn new() -> Self {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        kernel.open("e2e",OpenSource::Untitled(pm_model::json::parse(&json!({
            "id":"e2e","name":"Synthetic","compId":"main","w":320,"h":180,"fps":30,"dur":12,"layers":[],"comps":{},"assets":{}
        }).to_string()).unwrap())).wait().unwrap().unwrap();
        Self {
            host: ActorAutomation::new(kernel),
            _actor: actor,
        }
    }
    fn call(&self, call: impl Into<Call>) -> pm_kernel::Ack {
        let ack = self
            .host
            .kernel
            .call("e2e", Request::new(Caller::realm(71, 1, Origin::Mcp), call))
            .wait()
            .unwrap()
            .unwrap();
        assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
        ack
    }
    fn edit(&self, commands: Vec<Value>) {
        let before = self.doc();
        let batch = pm_mcp::commands::plan(&before, &[], &commands, 0.).unwrap();
        self.call(Call::Transact {
            label: Some("Synthetic e2e".into()),
            body: WriteBody::Commands(batch.commands),
            expected_revision: Some(self.host.inspect_project("e2e").unwrap().revision()),
        });
    }
    fn doc(&self) -> Value {
        self.host.read_document("e2e").unwrap()
    }
}
fn selection_preserves_redo() -> Check {
    let a = ActorFixture::new();
    a.edit(vec![
        json!({"type":"add_layer","id":"existing-layer","layerType":"shape"}),
    ]);
    a.edit(vec![json!({"type":"add_layer","id":"agent-title","layerType":"text","content":{"text":"Keep me"}})]);
    assert!(matches!(a.call(HistoryCall::Undo).reply, Reply::Bool(true)));
    assert_eq!(a.doc()["layers"].as_array().unwrap().len(), 1);
    a.call(SelectionCall::Layers {
        ids: vec!["existing-layer".into()],
        add: false,
    });
    assert!(matches!(a.call(HistoryCall::Redo).reply, Reply::Bool(true)));
    assert_eq!(layer(&a.doc(), "agent-title")["d"]["text"], "Keep me");
    assert_eq!(
        a.host.inspect_project("e2e").unwrap().selection.layers(),
        ["existing-layer"]
    );
    Ok(())
}
fn strip_move_and_library() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([
        {"type":"add_layer","id":"a","layerType":"text","from":2,"duration":3},
        {"type":"add_layer","id":"b","layerType":"shape","from":3,"duration":2}
    ]))?;
    commit(&mut f)?;
    start(&mut f)?;
    let before = f.document()?["layers"].clone();
    assert_eq!(
        f.call("get_timeline", json!({}))?["strips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    f.call(
        "edit_layer_strips",
        json!({"operation":"move","layerIds":["a","b"],"at":5}),
    )?;
    let doc = f.document()?;
    assert_eq!(layer(&doc, "a")["from"], 5);
    assert_eq!(layer(&doc, "b")["from"], 6);
    commit(&mut f)?;
    undo(&f)?;
    assert_eq!(f.document()?["layers"], before);
    start(&mut f)?;
    f.call(
        "manage_project",
        json!({"operation":"rename","name":"Original film"}),
    )?;
    commit(&mut f)?;
    f.call(
        "start_session",
        json!({"context":"app","prompt":"Synthetic library"}),
    )?;
    let copy = f.call(
        "manage_project",
        json!({"operation":"duplicate","projectId":f.id,"name":"Film copy"}),
    )?;
    let created = f.call(
        "manage_project",
        json!({"operation":"create","name":"New film","w":640,"h":360,"fps":24,"dur":12}),
    )?;
    assert_ne!(copy["projectId"], f.id);
    assert_ne!(copy["projectId"], created["projectId"]);
    let list = f.call("manage_project", json!({"operation":"list"}))?;
    for (id, name) in [
        (json!(f.id), "Original film"),
        (copy["projectId"].clone(), "Film copy"),
        (created["projectId"].clone(), "New film"),
    ] {
        assert!(
            list["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == id && p["name"] == name)
        );
    }
    assert_eq!(
        f.host
            .lock()
            .unwrap()
            .read_document(created["projectId"].as_str().unwrap())
            .unwrap()["w"],
        640
    );
    Ok(())
}

fn video() -> Result<Fixture, String> {
    let mut f = Fixture::new()?;
    f.call("finish_session", json!({"commit":true}))?;
    let mut p = f.document()?;
    p["dur"] = json!(30);
    let source = f.scratch.0.join("synthetic-source.bin");
    std::fs::write(&source, b"synthetic immutable media").map_err(|e| e.to_string())?;
    // A video asset without `timing` is a v1 one: opening it adds the Electron
    // video reading marker (`videoColor`), so the document is shaped as loaded.
    p["assets"] = json!({"footage":{"id":"footage","kind":"video","name":"Synthetic numbered frames","dur":30,"sourcePath":source,"unknown":{"retain":true},"videoColor":"electron"}});
    let rev = f.revision();
    f.host
        .lock()
        .unwrap()
        .restore(&f.id, p, rev, 71)
        .map_err(|e| e.to_string())?;
    start(&mut f)?;
    f.edit(json!([{"type":"add_layer","id":"clip","layerType":"video","from":2,"duration":8,"content":{"asset":"footage","trim":3,"speed":1,"embeddedAudio":false}},
        {"type":"replace_keyframes","target":"clip","path":"opacity","keyframes":[{"time":0,"value":10},{"time":8,"value":90}]}]))?;
    commit(&mut f)?;
    start(&mut f)?;
    Ok(f)
}
fn speed(rate: f64) -> Check {
    let mut f = video()?;
    f.call(
        "edit_video",
        json!({"operation":"speed","layerId":"clip","speed":rate}),
    )?;
    let doc = f.document()?;
    let clip = layer(&doc, "clip");
    assert_eq!(clip["from"], 2);
    assert_eq!(clip["dur"].as_f64(), Some(8. / rate));
    assert_eq!(scalar(&clip["d"]["trim"]), 3.);
    assert_eq!(scalar(&clip["d"]["speed"]), rate);
    assert_eq!(clip["p"]["opacity"]["kf"][1]["t"].as_f64(), Some(8. / rate));
    verify_saved(&mut f)?;
    Ok(())
}
fn speed_double() -> Check {
    speed(2.)
}
fn speed_half() -> Check {
    speed(0.5)
}
fn scalar(v: &Value) -> f64 {
    v["v"]
        .as_f64()
        .or_else(|| v.as_f64())
        .expect("numeric property")
}
fn speed_undo_redo() -> Check {
    let mut f = video()?;
    let before = f.document()?["layers"].clone();
    f.call(
        "edit_video",
        json!({"operation":"speed","layerId":"clip","speed":2}),
    )?;
    let after = f.document()?["layers"].clone();
    commit(&mut f)?;
    let mut saved = store(&mut f)?;
    saved.undo().map_err(|e| e.to_string())?;
    assert_eq!(document(&saved)["layers"], before);
    saved.redo().map_err(|e| e.to_string())?;
    assert_eq!(document(&saved)["layers"], after);
    Ok(())
}
fn video_split_continuity() -> Check {
    let mut f = video()?;
    let before = f.document()?;
    f.call(
        "edit_video",
        json!({"operation":"split","layerId":"clip","at":6}),
    )?;
    let after = f.document()?;
    let left = layer(&after, "clip");
    let right = after["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] != "clip")
        .unwrap();
    assert_eq!(left["from"], 2);
    assert_eq!(left["dur"], 4);
    assert_eq!(right["from"], 6);
    assert_eq!(right["dur"], 4);
    assert_eq!(right["d"]["trim"]["v"], 7);
    // Composition-time opacity must be continuous on both sides of the cut.
    for (id, t) in [("clip", 5.5), (right["id"].as_str().unwrap(), 6.5)] {
        assert_eq!(
            evaluate(after.clone(), id, "opacity", t),
            evaluate(before.clone(), "clip", "opacity", t)
        );
    }
    f.call("rollback_changes", json!({}))?;
    assert_eq!(f.document()?["layers"], before["layers"]);
    Ok(())
}
fn video_slip_bounds() -> Check {
    let mut f = video()?;
    let before = f.document()?;
    f.call(
        "edit_video",
        json!({"operation":"slip","layerId":"clip","sourceIn":5}),
    )?;
    let after = f.document()?;
    for k in ["from", "dur", "p"] {
        assert_eq!(layer(&after, "clip")[k], layer(&before, "clip")[k]);
    }
    let snapshot = after.clone();
    tool_error(
        &mut f.session,
        "edit_video",
        json!({"operation":"slip","layerId":"clip","sourceIn":29}),
    )?;
    assert_eq!(f.document()?, snapshot);
    Ok(())
}
fn video_trim_bounds() -> Check {
    let mut f = video()?;
    let before = f.document()?;
    tool_error(
        &mut f.session,
        "edit_video",
        json!({"operation":"trim","layerId":"clip","start":0,"end":35}),
    )?;
    assert_eq!(f.document()?, before);
    f.call(
        "edit_video",
        json!({"operation":"trim","layerId":"clip","start":4,"end":9}),
    )?;
    let doc = f.document()?;
    let clip = layer(&doc, "clip");
    assert_eq!(clip["from"], 4);
    assert_eq!(clip["dur"], 5);
    assert_eq!(clip["d"]["trim"]["v"], 5);
    for t in [4., 6., 8.5] {
        assert_eq!(
            evaluate(doc.clone(), "clip", "opacity", t),
            evaluate(before.clone(), "clip", "opacity", t)
        );
    }
    Ok(())
}
fn keyframe_drag_values() -> Check {
    for (path, values) in [
        ("c.color", json!(["#ff0000", "#0000ff"])),
        ("position.y", json!([100, 300])),
    ] {
        let mut f = Fixture::new()?;
        f.edit(json!([{"type":"add_layer","id":"keys","layerType":"text"},
            {"type":"replace_keyframes","target":"keys","path":path,"keyframes":[{"time":1,"value":values[0]},{"time":3,"value":values[1]}]}]))?;
        commit(&mut f)?;
        let before = f.document()?;
        let keys = if path == "c.color" {
            &layer(&before, "keys")["d"]["color"]["kf"]
        } else {
            &layer(&before, "keys")["p"][path]["kf"]
        };
        let keys = keys.as_array().unwrap();
        let ids = keys
            .iter()
            .map(|k| k["i"].as_str().unwrap().into())
            .collect();
        foreign(
            &f,
            vec![
                pm_kernel::KeyframeCommand::Move {
                    key_ids: ids,
                    delta: 1.,
                }
                .into(),
            ],
        )?;
        let after = f.document()?;
        let moved = if path == "c.color" {
            &layer(&after, "keys")["d"]["color"]["kf"]
        } else {
            &layer(&after, "keys")["p"][path]["kf"]
        };
        for (a, b) in keys.iter().zip(moved.as_array().unwrap()) {
            let mut expected = a.clone();
            expected["t"] = json!(a["t"].as_f64().unwrap() + 1.);
            // Compare every authored non-time field, not only numeric values.
            let mut actual = b.clone();
            actual["t"] = expected["t"].clone();
            assert_eq!(expected, actual);
            assert_eq!(b["t"].as_f64(), expected["t"].as_f64());
        }
        undo(&f)?;
        assert_eq!(f.document()?["layers"], before["layers"]);
    }
    Ok(())
}
fn markers_persist() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([{"type":"add_marker","time":1.5,"name":"Cue α"},{"type":"add_marker","time":3,"name":"End"}]))?;
    commit(&mut f)?;
    let before = f.document()?["markers"].clone();
    assert_eq!(before[0]["t"], 1.5);
    assert_eq!(before[0]["name"], "Cue α");
    let mut saved = store(&mut f)?;
    assert_eq!(document(&saved)["markers"], before);
    saved.undo().map_err(|e| e.to_string())?;
    assert_eq!(document(&saved)["markers"], json!([]));
    saved.redo().map_err(|e| e.to_string())?;
    assert_eq!(document(&saved)["markers"], before);
    Ok(())
}
fn live_drag(cancel: bool) -> Check {
    let a = ActorFixture::new();
    a.edit(vec![
        json!({"type":"add_layer","id":"drag","layerType":"solid","from":2,"duration":3}),
    ]);
    let before = without_revision(a.doc());
    let Reply::Gesture(id) = a
        .call(pm_kernel::GestureCall::Begin {
            label: "Move clip".into(),
        })
        .reply
    else {
        panic!("gesture id")
    };
    for from in [4., 5.] {
        let batch = pm_mcp::commands::plan(
            &a.doc(),
            &[],
            &[json!({"type":"set_layer","target":"drag","patch":{"from":from}})],
            0.,
        )
        .unwrap();
        a.call(pm_kernel::GestureCall::Step {
            gesture: id,
            body: WriteBody::Commands(batch.commands),
            replace: true,
        });
        assert_eq!(layer(&a.doc(), "drag")["from"].as_f64(), Some(from));
    }
    if cancel {
        a.call(pm_kernel::GestureCall::Cancel { gesture: id });
        assert_eq!(without_revision(a.doc()), before);
    } else {
        a.call(pm_kernel::GestureCall::Commit { gesture: id });
        a.call(HistoryCall::Undo);
        assert_eq!(without_revision(a.doc()), before);
        a.call(HistoryCall::Redo);
        assert_eq!(layer(&a.doc(), "drag")["from"], 5);
    }
    Ok(())
}
fn live_drag_commit() -> Check {
    live_drag(false)
}
fn live_drag_cancel() -> Check {
    live_drag(true)
}
fn authored_fractional_rates() -> Check {
    for rate in [23.976, 29.97, 59.94, 30000. / 1001.] {
        let mut f = Fixture::new()?;
        f.edit(json!([{"type":"set_composition","patch":{"fps":rate,"width":641,"height":359,"duration":12.5}}]))?;
        let p = f.document()?;
        assert_eq!(p["fps"], json!(rate));
        assert_eq!(p["w"], 641);
        assert_eq!(p["h"], 359);
        assert_eq!(p["dur"], 12.5);
        verify_saved(&mut f)?;
    }
    Ok(())
}
fn portable_opaque_data() -> Check {
    let mut f = Fixture::new()?;
    commit(&mut f)?;
    let mut doc = f.document()?;
    doc["unknownExtensionData"] = json!({"unicode":"👩🏽‍💻\nمرحبا\u{0000}end","revision":7,"unknown":[null,false,1.2345678901234567]});
    let revision = f.revision();
    f.host
        .lock()
        .unwrap()
        .restore(&f.id, doc, revision, 71)
        .map_err(|e| e.to_string())?;
    start(&mut f)?;
    f.add("portable")?;
    commit(&mut f)?;
    verify_saved(&mut f)?;
    Ok(())
}
fn portable_redo_tail() -> Check {
    let mut f = Fixture::new()?;
    f.add("first")?;
    commit(&mut f)?;
    start(&mut f)?;
    f.add("second")?;
    commit(&mut f)?;
    undo(&f)?;
    let before = f.document()?["layers"].clone();
    let mut saved = store(&mut f)?;
    assert_eq!(document(&saved)["layers"], before);
    assert!(saved.can_redo());
    saved.redo().map_err(|e| e.to_string())?;
    assert!(
        document(&saved)["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["id"] == "second")
    );
    saved.undo().map_err(|e| e.to_string())?;
    assert_eq!(document(&saved)["layers"], before);
    Ok(())
}
fn library_trash_retains_work() -> Check {
    let mut f = Fixture::new()?;
    f.add("retained")?;
    commit(&mut f)?;
    f.call(
        "start_session",
        json!({"context":"app","prompt":"Synthetic archive"}),
    )?;
    // Switch to another project so the library can archive this closed slot.
    f.call(
        "manage_project",
        json!({"operation":"create","name":"Synthetic neighbor"}),
    )?;
    f.call(
        "manage_project",
        json!({"operation":"trash","projectId":f.id}),
    )?;
    let listed = f.call("manage_project", json!({"operation":"list"}))?;
    assert!(
        listed["trash"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == f.id)
    );
    f.call(
        "manage_project",
        json!({"operation":"restore","projectId":f.id}),
    )?;
    f.call(
        "manage_project",
        json!({"operation":"open","projectId":f.id}),
    )?;
    assert_eq!(layer(&f.document()?, "retained")["id"], "retained");
    Ok(())
}
fn media_folder_undo() -> Check {
    let mut f = video()?;
    let folder = f.call(
        "manage_media",
        json!({"operation":"create_folder","name":"Recovered"}),
    )?["folderId"]
        .clone();
    f.call(
        "manage_media",
        json!({"operation":"move","assetIds":["footage"],"folderId":folder}),
    )?;
    f.call(
        "manage_media",
        json!({"operation":"rename","assetIds":["footage"],"name":"Music"}),
    )?;
    assert_eq!(f.document()?["assets"]["footage"]["name"], "Music");
    undo(&f)?;
    assert_eq!(
        f.document()?["assets"]["footage"]["name"],
        "Synthetic numbered frames"
    );
    f.call("list_media", json!({}))?;
    f.call(
        "manage_media",
        json!({"operation":"delete_folder","folderId":folder}),
    )?;
    assert!(f.document()?["assets"]["footage"]["folder"].is_null());
    assert_eq!(
        f.document()?["assets"]["footage"]["unknown"]["retain"],
        true
    );
    Ok(())
}
fn media_delete_lock() -> Check {
    let mut f = video()?;
    // A real ordinary file is retained even when its project metadata is removed.
    let source = f.scratch.0.join("synthetic-source.bin");
    std::fs::write(&source, b"synthetic immutable media").map_err(|e| e.to_string())?;
    f.edit(json!([{"type":"set_layer","target":"clip","patch":{"locked":true}}]))?;
    let before = f.document()?;
    let err = tool_error(
        &mut f.session,
        "manage_media",
        json!({"operation":"delete","assetIds":["footage"],"removeReferenced":true}),
    )?;
    assert!(err["message"].as_str().unwrap().contains("Unlock"));
    assert_eq!(f.document()?, before);
    commit(&mut f)?;
    undo(&f)?;
    start(&mut f)?;
    let before = f.document()?;
    tool_error(
        &mut f.session,
        "manage_media",
        json!({"operation":"delete","assetIds":["footage"]}),
    )?;
    assert_eq!(f.document()?, before);
    f.call(
        "manage_media",
        json!({"operation":"delete","assetIds":["footage"],"removeReferenced":true}),
    )?;
    assert!(f.document()?["assets"]["footage"].is_null());
    assert!(f.document()?["layers"].as_array().unwrap().is_empty());
    undo(&f)?;
    assert_eq!(f.document()?["assets"], before["assets"]);
    assert_eq!(f.document()?["layers"], before["layers"]);
    assert_eq!(std::fs::read(source).unwrap(), b"synthetic immutable media");
    Ok(())
}
fn caption(format: &str) -> Check {
    let mut f = Fixture::new()?;
    commit(&mut f)?;
    let mut p = f.document()?;
    p["layers"] = json!([{"id":"captions","type":"captions","from":2,"dur":4,"d":{"language":"en","cues":[{"id":"cue","start":0.25,"end":1.5,"text":"Hello & world"}]}}]);
    let rev = f.revision();
    f.host
        .lock()
        .unwrap()
        .restore(&f.id, p, rev, 71)
        .map_err(|e| e.to_string())?;
    start(&mut f)?;
    let before = f.document()?;
    let result = f.call(
        "export_captions",
        json!({"layerId":"captions","format":format}),
    )?;
    let expected = if format == "srt" {
        "1\n00:00:02,250 --> 00:00:03,500\nHello & world\n"
    } else {
        "WEBVTT\nLanguage: en\n\n00:00:02.250 --> 00:00:03.500\nHello &amp; world\n"
    };
    assert_eq!(result["text"], expected);
    assert_eq!(f.document()?, before);
    Ok(())
}
fn caption_srt() -> Check {
    caption("srt")
}
fn caption_vtt() -> Check {
    caption("vtt")
}

#[derive(Default)]
struct CaptureDouble {
    calls: Mutex<Vec<CaptureRequest>>,
    fail: bool,
}
impl EngineHook for CaptureDouble {
    fn capture(&self, r: &CaptureRequest) -> pm_mcp::Result<Value> {
        self.calls.lock().unwrap().push(r.clone());
        if self.fail {
            Err(pm_mcp::Error::unavailable("synthetic capture failure"))
        } else {
            Ok(json!({"testDouble":true,"times":r.times,"revision":r.revision}))
        }
    }
}
fn capture(fail: bool, stale: bool) -> Check {
    let mut f = Fixture::new()?;
    f.add("kept")?;
    let double = Arc::new(CaptureDouble {
        fail,
        ..Default::default()
    });
    f.host.lock().unwrap().engine = Some(double.clone());
    let request = CaptureRequest {
        project_id: f.id.clone(),
        revision: f.revision(),
        times: vec![0., 1.5, 3.],
        width: 320,
    };
    if stale {
        f.add("newer")?;
    }
    let before = f.document()?;
    let response = f.host.lock().unwrap().capture_frames(&request);
    if stale {
        assert_eq!(response.unwrap_err().code, "revision_conflict");
        assert!(double.calls.lock().unwrap().is_empty());
    } else if fail {
        assert_eq!(response.unwrap_err().code, "not_available_in_this_build");
    } else {
        assert_eq!(response.unwrap()["testDouble"], true);
        assert_eq!(*double.calls.lock().unwrap(), vec![request]);
    }
    assert_eq!(f.document()?, before);
    commit(&mut f)?;
    verify_saved(&mut f)?;
    Ok(())
}
fn capture_preserves_document() -> Check {
    capture(false, false)
}
fn capture_rejects_stale() -> Check {
    capture(false, true)
}
fn capture_failure_preserves_edits() -> Check {
    capture(true, false)
}

struct SurfaceDouble {
    actions: Mutex<Vec<Value>>,
}
impl SurfaceHook for SurfaceDouble {
    fn call(&self, tool: &str, _: Option<&str>, args: &Value) -> pm_mcp::Result<Value> {
        if tool != "interact_panel"
            || args["panelId"] != "synthetic.panel"
            || args["ref"] != "stable.play"
        {
            return Err(pm_mcp::Error::invalid("unknown stable id"));
        }
        self.actions.lock().unwrap().push(args["action"].clone());
        Ok(json!({"testDouble":true}))
    }
}
fn surface(unknown: bool) -> Check {
    let mut f = Fixture::new()?;
    let double = Arc::new(SurfaceDouble {
        actions: Mutex::new(vec![]),
    });
    f.host.lock().unwrap().surfaces = Some(double.clone());
    let before = without_revision(f.document()?);
    let element = if unknown { "child-0" } else { "stable.play" };
    let reply = f.call(
        "interact_panel",
        json!({"panelId":"synthetic.panel","ref":element,"action":"click"}),
    );
    if unknown {
        assert!(reply.is_err());
        assert!(double.actions.lock().unwrap().is_empty());
    } else {
        reply?;
        assert_eq!(*double.actions.lock().unwrap(), vec![json!("click")]);
    }
    assert_eq!(without_revision(f.document()?), before);
    Ok(())
}
fn surface_stable_ids() -> Check {
    surface(false)
}
fn surface_unknown_ids() -> Check {
    surface(true)
}
fn mcp_tool_contract() -> Check {
    let f = Fixture::new()?;
    assert_eq!(f.session.tools(), json!(pm_mcp::catalog::served_tools()));
    assert_eq!(f.session.tools().as_array().unwrap().len(), 70);
    assert_eq!(
        pm_mcp::catalog::tools().len(),
        68,
        "the pinned main catalog"
    );
    assert_eq!(
        pm_mcp::catalog::legacy_catalog()["tools"]
            .as_array()
            .unwrap()
            .len(),
        67
    );
    Ok(())
}
fn pending_tools_are_explicit() -> Check {
    let mut f = Fixture::new()?;
    let before = f.document()?;
    let revision = f.revision();
    for spec in pm_mcp::catalog::tools() {
        let name = spec["name"].as_str().unwrap();
        if pm_mcp::catalog::IMPLEMENTED.contains(&name) {
            continue;
        }
        let args = crate::scenarios::pending_arguments(&spec["inputSchema"])?;
        if name == "project_memory" {
            assert_eq!(f.call(name, args)?, json!({"memory":[],"total":0}));
            let error = tool_error(
                &mut f.session,
                name,
                json!({
                    "operation":"add", "kind":"goal", "text":"Synthetic durable goal"
                }),
            )?;
            assert_eq!(error["code"], "not_available_in_this_build");
            assert_eq!(
                error["backing"],
                "kernel durable project-memory metadata admission without edit revision/Undo; preserve memory across rollback"
            );
            continue;
        }
        let error = tool_error(&mut f.session, name, args)?;
        assert_eq!(error["code"], "not_available_in_this_build");
        assert_eq!(error["backing"], pm_mcp::catalog::backing(name));
    }
    assert_eq!(f.document()?, before);
    assert_eq!(f.revision(), revision);
    Ok(())
}
