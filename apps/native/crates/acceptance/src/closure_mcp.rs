//! Wave 5: execute merged MCP bodies. Synthetic metadata is not decoded media.
use crate::fixture::{Fixture, layer, tool_error, without_revision};
use pm_mcp::media::{MediaRegistry, MediaState};
use serde_json::{Value, json};
use std::sync::Arc;
type Check = Result<(), String>;
fn scalar(value: &Value) -> Value {
    if value.is_object() {
        value["v"].clone()
    } else {
        value.clone()
    }
}

fn restore(f: &mut Fixture, mut document: Value) -> Check {
    let revision = f.revision();
    document["revision"] = json!(revision);
    f.host
        .lock()
        .unwrap()
        .restore(&f.id, document, revision, 211)
        .map_err(|e| e.to_string())?;
    f.call("get_project_state", json!({}))?;
    Ok(())
}
fn seed_video(f: &mut Fixture, kind: &str) -> Check {
    let mut document = f.document()?;
    document["assets"] = json!({"source":{"id":"source","kind":"video","name":"Synthetic source","dur":12,"hasAudio":true,"persisted":true,"unknown":{"keep":7}}});
    restore(f, document)?;
    f.edit(json!([
        {"type":"add_layer","id":"clip","layerType":kind,"from":2,"duration":3,"content":if kind=="audio" {json!({"asset":"source","trim":1,"gain":1,"fadeIn":0,"fadeOut":0})} else {json!({"asset":"source","trim":1,"speed":2,"embeddedAudio":false})}}
    ]))?;
    if kind != "audio" {
        f.edit(json!([{ "type":"replace_keyframes","target":"clip","path":"opacity","keyframes":[{"id":"authored-key","time":1,"value":25}]}]))?;
    }
    f.call("finish_session", json!({"commit":true}))?;
    f.call("start_session", json!({"prompt":"Synthetic edit"}))?;
    Ok(())
}
/// Persist the actual MCP edit's history, then exercise both directions through
/// the production PMV reader, not a separately reconstructed command.
fn persisted_history(f: &mut Fixture, before: Value) -> Check {
    let after = without_revision(f.document()?);
    assert_ne!(after, without_revision(before.clone()));
    let saved = f.call("save_project", json!({}))?;
    let opened = pm_kernel::project_io::open_project(
        std::path::Path::new(saved["path"].as_str().ok_or("save path missing")?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let mut store = pm_kernel::DocumentStore::open(opened);
    let document = |store: &pm_kernel::DocumentStore| -> Value {
        let value =
            pm_model::canonical::realize_project(store.snapshot().document.to_value().unwrap());
        without_revision(
            serde_json::from_str(&pm_model::json::to_canonical_string(&value)).unwrap(),
        )
    };
    assert_eq!(document(&store), after);
    store.undo().map_err(|e| e.to_string())?;
    assert_eq!(document(&store), without_revision(before));
    store.redo().map_err(|e| e.to_string())?;
    assert_eq!(document(&store), after);
    Ok(())
}

pub fn video() -> Check {
    for args in [
        json!({"operation":"insert","assetId":"source","from":6,"duration":2,"sourceIn":3}),
        json!({"operation":"split","layerId":"clip","at":3}),
        json!({"operation":"trim","layerId":"clip","start":2.5,"end":4.5}),
        json!({"operation":"move","layerId":"clip","from":5}),
        json!({"operation":"slip","layerId":"clip","sourceIn":5}),
        json!({"operation":"speed","layerId":"clip","speed":1}),
        json!({"operation":"separate_audio","layerId":"clip"}),
        json!({"operation":"remove","layerId":"clip"}),
        json!({"operation":"audio","layerId":"clip","gain":0.5,"fadeIn":0.25,"fadeOut":0.5}),
    ] {
        let mut f = Fixture::new()?;
        seed_video(
            &mut f,
            if args["operation"] == "audio" {
                "audio"
            } else {
                "video"
            },
        )?;
        if args["operation"] == "separate_audio" {
            let mut document = f.document()?;
            document["layers"][0]["d"]["embeddedAudio"] = json!(true);
            document["layers"][0]["d"]["speed"] = json!(1);
            restore(&mut f, document)?;
            f.call("finish_session", json!({"commit":true}))?;
            f.call("start_session", json!({"prompt":"Synthetic separation"}))?;
        }
        let before = f.document()?;
        f.call("edit_video", args.clone())?;
        let after = f.document()?;
        match args["operation"].as_str().unwrap() {
            "insert" => {
                assert_eq!(after["layers"].as_array().unwrap().len(), 2);
                let inserted = after["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|l| l["id"] != "clip")
                    .unwrap();
                assert_eq!(inserted["from"], 6);
                assert_eq!(inserted["dur"], 2);
                assert_eq!(inserted["d"]["trim"]["v"], 3);
            }
            "split" => {
                let layers = after["layers"].as_array().unwrap();
                assert_eq!(layers.len(), 2);
                assert_eq!(layer(&after, "clip")["dur"], 1);
                let tail = layers.iter().find(|l| l["id"] != "clip").unwrap();
                assert_eq!(tail["from"], 3);
                assert_eq!(tail["dur"], 2);
                assert_eq!(tail["d"]["trim"]["v"], 3);
                assert_eq!(tail["p"]["opacity"]["kf"][0]["t"], 0);
            }
            "trim" => {
                assert_eq!(layer(&after, "clip")["from"], 2.5);
                assert_eq!(layer(&after, "clip")["dur"], 2);
                assert_eq!(layer(&after, "clip")["d"]["trim"]["v"], 2);
                assert_eq!(layer(&after, "clip")["p"]["opacity"]["kf"][0]["t"], 0.5);
            }
            "move" => assert_eq!(layer(&after, "clip")["from"], 5),
            "slip" => {
                assert_eq!(layer(&after, "clip")["d"]["trim"]["v"], 5);
                assert_eq!(layer(&after, "clip")["p"], layer(&before, "clip")["p"]);
                assert_eq!(layer(&after, "clip")["from"], 2);
                assert_eq!(layer(&after, "clip")["dur"], 3);
            }
            "speed" => {
                assert_eq!(layer(&after, "clip")["dur"], 6);
                assert_eq!(scalar(&layer(&after, "clip")["d"]["speed"]), 1);
                assert_eq!(
                    layer(&after, "clip")["d"]["trim"],
                    layer(&before, "clip")["d"]["trim"]
                );
            }
            "separate_audio" => assert!(
                after["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|l| l["type"] == "audio")
            ),
            "remove" => assert!(after["layers"].as_array().unwrap().is_empty()),
            "audio" => assert_eq!(scalar(&layer(&after, "clip")["d"]["gain"]), 0.5),
            _ => unreachable!(),
        }
        assert_eq!(after["assets"], before["assets"]);
        persisted_history(&mut f, before)?;
    }
    video_refusals()
}

pub fn video_refusals() -> Check {
    let mut f = Fixture::new()?;
    seed_video(&mut f, "video")?;
    for args in [
        json!({"operation":"slip","layerId":"clip","sourceIn":8}),
        json!({"operation":"split","layerId":"clip","at":2}),
        json!({"operation":"trim","layerId":"clip","start":0,"end":5}),
        json!({"operation":"move","layerId":"missing","from":5}),
        json!({"operation":"insert","assetId":"missing","from":0,"duration":1}),
    ] {
        let before = f.document()?;
        tool_error(&mut f.session, "edit_video", args)?;
        assert_eq!(f.document()?, before);
    }
    f.edit(json!([{"type":"set_layer","target":"clip","patch":{"locked":true}}]))?;
    let before = f.document()?;
    tool_error(
        &mut f.session,
        "edit_video",
        json!({"operation":"move","layerId":"clip","from":5}),
    )?;
    assert_eq!(f.document()?, before);
    Ok(())
}

pub fn slip() -> Check {
    let mut f = Fixture::new()?;
    seed_video(&mut f, "video")?;
    let before = f.document()?;
    f.call(
        "edit_video",
        json!({"operation":"slip","layerId":"clip","sourceIn":5}),
    )?;
    let after = f.document()?;
    assert_eq!(layer(&after, "clip")["d"]["trim"]["v"], 5);
    for key in ["from", "dur", "p"] {
        assert_eq!(layer(&after, "clip")[key], layer(&before, "clip")[key]);
    }
    tool_error(
        &mut f.session,
        "edit_video",
        json!({"operation":"slip","layerId":"clip","sourceIn":8}),
    )?;
    assert_eq!(f.document()?, after);
    persisted_history(&mut f, before)
}

pub fn strips() -> Check {
    for args in [
        json!({"operation":"move","layerIds":["one","two"],"delta":1}),
        json!({"operation":"duplicate","layerIds":["one"],"delta":1}),
        json!({"operation":"sequence","layerIds":["one","two"],"at":1,"gap":1}),
        json!({"operation":"align","layerIds":["one","two"],"at":5,"edge":"end"}),
        json!({"operation":"reorder","layerIds":["one"],"anchorId":"two","placement":"before"}),
        json!({"operation":"state","layerIds":["one","two"],"patch":{"name":"Authored name","solo":true}}),
        json!({"operation":"remove","layerIds":["one"]}),
        json!({"operation":"close_gap","layerIds":["two"],"start":2,"end":4}),
    ] {
        let mut f = Fixture::new()?;
        f.edit(json!([
            {"type":"add_layer","id":"one","layerType":"solid","from":0,"duration":2},
            {"type":"add_layer","id":"two","layerType":"solid","from":4,"duration":2}
        ]))?;
        f.call("finish_session", json!({"commit":true}))?;
        f.call("start_session", json!({"prompt":"Synthetic strips"}))?;
        let before = f.document()?;
        let mut invalid = args.clone();
        invalid["layerIds"] = json!(["one", "missing"]);
        tool_error(&mut f.session, "edit_layer_strips", invalid)?;
        assert_eq!(f.document()?, before);
        f.call("edit_layer_strips", args.clone())?;
        let after = f.document()?;
        match args["operation"].as_str().unwrap() {
            "move" => {
                assert_eq!(layer(&after, "one")["from"], 1);
                assert_eq!(layer(&after, "two")["from"], 5);
            }
            "duplicate" => assert_eq!(after["layers"].as_array().unwrap().len(), 3),
            "sequence" => {
                assert_eq!(layer(&after, "one")["from"], 1);
                assert_eq!(layer(&after, "two")["from"], 4);
            }
            "align" => {
                assert_eq!(layer(&after, "one")["from"], 3);
                assert_eq!(layer(&after, "two")["from"], 3);
            }
            "reorder" => assert_eq!(after["layers"][0]["id"], "one"),
            "state" => {
                for id in ["one", "two"] {
                    assert_eq!(layer(&after, id)["name"], "Authored name");
                    assert_eq!(layer(&after, id)["solo"], true);
                }
            }
            "remove" => assert_eq!(after["layers"].as_array().unwrap().len(), 1),
            "close_gap" => assert_eq!(layer(&after, "two")["from"], 2),
            _ => unreachable!(),
        }
        persisted_history(&mut f, before)?;
    }
    Ok(())
}

struct Registry;
impl MediaRegistry for Registry {
    fn state(&self, _: &str, asset: &str) -> MediaState {
        match asset {
            "available" => MediaState {
                available: true,
                duration: Some(12.),
                audio_duration: Some(12.),
                peaks: vec![0.75; 12],
                has_audio: true,
                ..Default::default()
            },
            "loading" => MediaState {
                loading: true,
                ..Default::default()
            },
            "failed" => MediaState {
                error: Some("synthetic decode failure".into()),
                ..Default::default()
            },
            "cloud" => MediaState {
                cloud: true,
                cloud_provider: Some("synthetic".into()),
                ..Default::default()
            },
            _ => MediaState::default(),
        }
    }
}
pub fn list_media() -> Check {
    let mut f = Fixture::new()?;
    let mut document = f.document()?;
    for id in ["available", "loading", "failed", "cloud", "missing"] {
        document["assets"][id] = json!({"id":id,"name":id,"kind":"video","dur":12,"persisted":true,"sourcePath":"/synthetic/original.mov","fingerprint":"synthetic-fingerprint","folder":"shots"});
    }
    document["assetFolders"] = json!({"shots":{"name":"Shots","parent":null}});
    document["layers"] = json!([
        {"id":"parent","type":"group","lock":true,"from":0,"dur":12},
        {"id":"nested-ref","type":"extension","group":"parent","from":0,"dur":12,"d":{"data":{"material":{"maps":{"color":"available"}}}}}
    ]);
    document["comps"] = json!({"child":{"id":"child","layers":[{"id":"child-ref","type":"video","d":{"asset":"available"},"from":0,"dur":3}]}});
    restore(&mut f, document)?;
    f.host.lock().unwrap().media_registry = Some(Arc::new(Registry));
    let before = f.document()?;
    let result = f.call("list_media", json!({}))?;
    assert_eq!(result["total"], 5);
    for id in ["available", "loading", "failed", "cloud", "missing"] {
        let result = f.call("list_media", json!({"assetId":id}))?;
        assert_eq!(result["assets"][0]["status"], id);
        assert_eq!(result["assets"][0]["persisted"], true);
        assert_eq!(result["assets"][0]["sourcePath"], "/synthetic/original.mov");
        assert_eq!(result["assets"][0]["folder"], "shots");
        if id == "available" {
            assert_eq!(result["assets"][0]["referenceCount"], 2);
            assert_eq!(result["assets"][0]["lockedReferences"], 1);
        }
    }
    assert_eq!(
        f.call("list_media", json!({"missingOnly":true}))?["total"],
        3
    );
    let first = f.call("list_media", json!({"limit":2}))?;
    assert_eq!(first["nextOffset"], 2);
    assert_eq!(
        f.call("list_media", json!({"offset":4,"limit":2}))?["nextOffset"],
        Value::Null
    );
    f.host.lock().unwrap().media_registry = None;
    assert_eq!(
        f.call("list_media", json!({"assetId":"available"}))?["assets"][0]["status"],
        "missing"
    );
    assert_eq!(f.document()?, before);
    Ok(())
}

pub fn manage_media() -> Check {
    let mut f = Fixture::new()?;
    seed_video(&mut f, "video")?;
    let source = f.scratch.0.join("synthetic-source.bytes");
    std::fs::write(&source, b"synthetic original bytes").map_err(|e| e.to_string())?;
    let mut document = f.document()?;
    document["assets"]["source"]["sourcePath"] = json!(source);
    restore(&mut f, document)?;
    let parent = f.call(
        "manage_media",
        json!({"operation":"create_folder","name":"Parent"}),
    )?["folderId"]
        .clone();
    let folder = f.call(
        "manage_media",
        json!({"operation":"create_folder","name":"Shots"}),
    )?["folderId"]
        .clone();
    for args in [
        json!({"operation":"rename","assetIds":["source"],"name":"Authored source"}),
        json!({"operation":"move","assetIds":["source"],"folderId":folder}),
        json!({"operation":"rename_folder","folderId":folder,"name":"Scene"}),
        json!({"operation":"move_folder","folderId":folder,"parentId":parent}),
        json!({"operation":"delete_folder","folderId":folder}),
        json!({"operation":"delete","assetIds":["source"],"removeReferenced":true}),
    ] {
        let before = f.document()?;
        if args["operation"] == "delete" {
            tool_error(
                &mut f.session,
                "manage_media",
                json!({"operation":"delete","assetIds":["source"]}),
            )?;
            assert_eq!(f.document()?, before);
        }
        f.call("manage_media", args.clone())?;
        let after = f.document()?;
        match args["operation"].as_str().unwrap() {
            "rename" => assert_eq!(after["assets"]["source"]["name"], "Authored source"),
            "move" => assert_eq!(after["assets"]["source"]["folder"], folder),
            "rename_folder" => assert_eq!(
                after["assetFolders"][folder.as_str().unwrap()]["name"],
                "Scene"
            ),
            "move_folder" => assert_eq!(
                after["assetFolders"][folder.as_str().unwrap()]["parent"],
                parent
            ),
            "delete_folder" => assert!(
                after["assetFolders"]
                    .get(folder.as_str().unwrap())
                    .is_none()
            ),
            "delete" => {
                assert!(after["assets"].get("source").is_none());
                assert!(after["layers"].as_array().unwrap().is_empty());
            }
            _ => unreachable!(),
        }
        if args["operation"] != "delete" {
            assert_eq!(after["assets"]["source"]["unknown"], json!({"keep":7}));
        }
        persisted_history(&mut f, before)?;
        assert_eq!(
            std::fs::read(&source).map_err(|e| e.to_string())?,
            b"synthetic original bytes"
        );
    }
    assert_eq!(
        tool_error(&mut f.session, "rollback_changes", json!({}))?["code"],
        "revision_conflict"
    );
    Ok(())
}

pub fn scene() -> Check {
    // Expected results are generated from the owner-selected Studio source,
    // independent of native code; tagged fixtures remain retained as evidence.
    let fixtures: Value =
        serde_json::from_str(include_str!("../data/fixtures/scene3d-source-read.json"))
            .map_err(|e| e.to_string())?;
    for fixture in fixtures["fixtures"].as_array().unwrap() {
        let mut f = Fixture::new()?;
        let mut document = f.document()?;
        for key in ["w", "h", "fps", "dur", "layers", "assets"] {
            document[key] = fixture["project"][key].clone();
        }
        restore(&mut f, document)?;
        let before = f.document()?;
        let mut expected = fixture["result"].clone();
        expected["revision"] = json!(f.revision());
        let actual = f.call("get_3d_scene", fixture["arguments"].clone())?;
        assert_close(&actual, &expected);
        assert_eq!(f.document()?, before);
    }
    Ok(())
}
fn assert_close(actual: &Value, expected: &Value) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => assert!(
            (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-9,
            "{actual} != {expected}"
        ),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_close(a, b);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len());
            for (k, b) in b {
                assert_close(a.get(k).expect("missing field"), b);
            }
        }
        _ => assert_eq!(actual, expected),
    }
}

pub fn captions() -> Check {
    let mut f = Fixture::new()?;
    let mut document = f.document()?;
    document["layers"] = json!([{"id":"captions","type":"captions","from":2,"dur":4,"d":{"language":"en-US","cues":[
        {"start":0,"end":2,"text":" Hello  & <world>\r\n\nagain "},
        {"start":1,"end":3,"text":"Second"},
        {"start":3.95,"end":5,"text":"clipped"},
        {"start":8,"end":9,"text":"outside"}
    ]}}]);
    restore(&mut f, document)?;
    let before = f.document()?;
    let srt = f.call(
        "export_captions",
        json!({"layerId":"captions","format":"srt"}),
    )?;
    assert_eq!(
        srt["text"],
        "1\n00:00:02,000 --> 00:00:03,000\nHello & <world>\nagain\n\n2\n00:00:03,000 --> 00:00:05,000\nSecond\n\n3\n00:00:05,950 --> 00:00:06,000\nclipped\n"
    );
    let vtt = f.call(
        "export_captions",
        json!({"layerId":"captions","format":"vtt"}),
    )?;
    assert!(
        vtt["text"]
            .as_str()
            .unwrap()
            .contains("00:00:02.000 --> 00:00:03.000\nHello &amp; &lt;world&gt;")
    );
    assert!(!vtt["text"].as_str().unwrap().contains("outside"));
    tool_error(
        &mut f.session,
        "export_captions",
        json!({"layerId":"missing"}),
    )?;
    assert_eq!(f.document()?, before);
    f.verify_saved()?;
    Ok(())
}

pub fn lint() -> Check {
    let mut f = Fixture::new()?;
    let mut document = f.document()?;
    document["work"] = json!([1, 10]);
    for id in ["available", "failed", "cloud", "missing"] {
        document["assets"][id] = json!({"id":id,"kind":"video","name":id,"dur":12});
    }
    document["layers"] = json!([
        {"id":"visible","type":"solid","name":"Visible","from":2,"dur":3,"on":true,"p":{"opacity":{"v":100,"kf":[]}}},
        {"id":"transparent","type":"solid","name":"Transparent","from":1,"dur":8,"on":true,"p":{"opacity":{"v":0,"kf":[]}}},
        {"id":"zero","type":"solid","name":"Zero","from":1,"dur":0},
        {"id":"outside","type":"solid","name":"Outside","from":20,"dur":1},
        {"id":"outside-work","type":"solid","name":"Outside work","from":10,"dur":1},
        {"id":"offline","type":"video","name":"Offline","from":1,"dur":3,"d":{"asset":"missing"}},
        {"id":"failed","type":"video","name":"Failed","from":1,"dur":3,"d":{"asset":"failed"}},
        {"id":"cloud","type":"video","name":"Cloud","from":1,"dur":3,"d":{"asset":"cloud"}},
        {"id":"unbound","type":"video","name":"Unbound","from":1,"dur":3,"d":{}},
        {"id":"missing-asset","type":"video","name":"Missing asset","from":1,"dur":3,"d":{"asset":"absent"}},
        {"id":"overrun","type":"audio","name":"Overrun","from":2,"dur":4,"d":{"asset":"available","trim":10}},
        {"id":"clipping","type":"audio","name":"Clipping","from":2,"dur":4,"d":{"asset":"available","trim":0,"gain":2}}
    ]);
    restore(&mut f, document)?;
    f.host.lock().unwrap().media_registry = Some(Arc::new(Registry));
    let before = f.document()?;
    let result = f.call("check_project", json!({}))?;
    let issues = result["issues"].as_array().unwrap();
    for (layer, code) in [
        ("transparent", "never-visible"),
        ("zero", "zero-duration"),
        ("outside", "outside-composition"),
        ("outside-work", "outside-work-area"),
        ("offline", "media-offline"),
        ("failed", "media-failed"),
        ("cloud", "media-in-cloud"),
        ("unbound", "no-media"),
        ("missing-asset", "missing-media"),
        ("overrun", "past-media-end"),
        ("clipping", "audio-clipping"),
    ] {
        assert!(
            issues
                .iter()
                .any(|issue| issue["layerId"] == layer && issue["code"] == code),
            "{layer}: missing {code}: {result}"
        );
    }
    let gaps = issues
        .iter()
        .find(|i| i["code"] == "empty-frames")
        .expect("missing empty-frame ranges");
    assert_eq!(gaps["ranges"], json!([[1, 2], [5, 10]]));
    assert_eq!(result["span"], json!([1, 10]));
    assert_eq!(result["mediaChecked"], true);
    assert_eq!(f.document()?, before);
    Ok(())
}

pub fn simplify() -> Check {
    for scope in ["project", "layers", "properties"] {
        let mut f = Fixture::new()?;
        for id in ["editable", "locked", "expression", "nonredundant"] {
            f.add(id)?;
            let values = if id == "nonredundant" || id == "expression" {
                [20, 80, 20]
            } else {
                [20, 20, 20]
            };
            f.edit(json!([{"type":"replace_keyframes","target":id,"path":"opacity","keyframes":[{"id":format!("{id}-first"),"time":0,"value":values[0]},{"id":format!("{id}-middle"),"time":1,"value":values[1]},{"id":format!("{id}-last"),"time":2,"value":values[2]}]}]))?;
        }
        let mut document = f.document()?;
        for layer in document["layers"].as_array_mut().unwrap() {
            if layer["id"] == "locked" {
                layer["lock"] = json!(true);
            }
            if layer["id"] == "expression" {
                layer["p"]["opacity"]["expr"] = json!("value + 1");
            }
        }
        restore(&mut f, document)?;
        f.call("finish_session", json!({"commit":true}))?;
        f.call(
            "start_session",
            json!({"prompt":"Synthetic simplification"}),
        )?;
        let mut command = json!({"type":"simplify_keyframes","scope":scope});
        if scope == "layers" {
            command["targets"] = json!(["editable", "expression", "nonredundant"]);
        }
        if scope == "properties" {
            command["properties"] = json!([{"layerId":"editable","path":"opacity"},{"layerId":"expression","path":"opacity"},{"layerId":"nonredundant","path":"opacity"}]);
        }
        let expressed = f.document()?;
        f.edit(json!([command.clone()]))?;
        assert_eq!(
            f.document()?,
            expressed,
            "opaque expressions fence simplification"
        );
        let mut document = f.document()?;
        for layer in document["layers"].as_array_mut().unwrap() {
            if layer["id"] == "expression" {
                layer["p"]["opacity"]["expr"] = Value::Null;
            }
        }
        restore(&mut f, document)?;
        f.call("finish_session", json!({"commit":true}))?;
        f.call(
            "start_session",
            json!({"prompt":"Synthetic verified simplification"}),
        )?;
        let before = f.document()?;
        let result = f.edit(json!([command]))?;
        let after = f.document()?;
        let keys = &layer(&after, "editable")["p"]["opacity"]["kf"];
        assert_eq!(keys.as_array().unwrap().len(), 2);
        assert_eq!(keys[0]["i"], "editable-first");
        assert_eq!(keys[1]["i"], "editable-last");
        for id in ["locked", "expression", "nonredundant"] {
            assert_eq!(layer(&after, id), layer(&before, id));
        }
        let data = &result["results"][0]["data"];
        assert_eq!(
            data["keysBefore"].as_u64().unwrap() - data["keysAfter"].as_u64().unwrap(),
            1
        );
        assert_eq!(data["propertiesChanged"], 1);
        persisted_history(&mut f, before)?;
    }
    Ok(())
}
