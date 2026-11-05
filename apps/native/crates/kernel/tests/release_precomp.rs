//! Release Pre-comp mirrors main's reversible carrier group and timing rules.
use pm_kernel::{Command, CompCommand, DocumentStore, Origin};
use pm_model::{Value, json::parse};

fn fixture() -> Value {
    parse(r##"{"id":"release-contract","name":"Release","compId":"root","w":128,"h":128,"fps":30,"dur":10,"bg":"#000000","assets":{},"layers":[
      {"id":"outer","type":"group","name":"Outer","from":0,"dur":10,"p":{}},
      {"id":"rig","type":"null","name":"Rig","from":0,"dur":10,"p":{}},
      {"id":"pre","type":"precomp","name":"Nested picture","from":3,"dur":3,"on":true,"shy":true,"solo":false,"color":4,"blend":{"v":"normal","kf":[]},"mblur":true,"parent":"rig","group":"outer","track":"track-1","matteSource":"matte","matteMode":"alpha","transitionIn":{"type":"synthetic","p":{}},"transitionOut":{"type":"synthetic","p":{}},"p":{"position.x":{"v":12,"kf":[{"i":"carrier-key","t":0,"v":12},{"i":"carrier-key2","t":3,"v":40}]},"opacity":{"v":80,"kf":[]}},"fx":[{"id":"carrier-fx","type":"synthetic","p":{"amount":{"v":1,"kf":[]}}}],"masks":[{"id":"carrier-mask","shape":{"v":[],"kf":[]}}],"d":{"comp":"sub","w":64,"h":64,"trim":{"v":1,"kf":[]},"speed":{"v":1,"kf":[]},"timeRemap":{"v":false,"kf":[]}}},
      {"id":"matte","type":"solid","name":"Matte","from":0,"dur":10,"p":{},"d":{"w":128,"h":128,"color":"#FFFFFF"}},
      {"id":"other","type":"precomp","name":"Other user","from":0,"dur":10,"p":{},"d":{"comp":"sub"}}
    ],"comps":{"sub":{"id":"sub","name":"Sub","w":64,"h":64,"fps":30,"dur":8,"layers":[
      {"id":"inner-group","type":"group","name":"Inner group","from":0,"dur":8,"p":{}},
      {"id":"clip","type":"video","name":"Clip","group":"inner-group","parent":"driver","matteSource":"mask","from":0,"dur":8,"on":true,"d":{"asset":"synthetic-video","trim":{"v":0.5,"kf":[]},"speed":2},"p":{"position.x":{"v":4,"kf":[{"i":"child-key","t":2,"v":12}]}},"fx":[{"id":"child-fx","type":"synthetic","inputs":{"map":{"layerId":"mask","space":"source","stage":"effects"},"external":{"layerId":"external","space":"composition","stage":"source"}},"p":{"gain":{"v":1,"kf":[{"i":"effect-key","t":1,"v":2}]}}}]},
      {"id":"driver","type":"null","name":"Driver","from":1,"dur":6,"p":{}},
      {"id":"mask","type":"solid","name":"Mask","from":1,"dur":6,"p":{},"d":{"w":64,"h":64,"color":"#FFFFFF"}},
      {"id":"hidden","type":"solid","name":"Outside span","from":7,"dur":1,"on":true,"parent":"clip","p":{}},
      {"id":"captions","type":"captions","name":"Captions","from":0,"dur":8,"p":{},"d":{"cues":[{"start":1,"end":2,"words":[{"start":1,"end":2}]}]}},
      {"id":"orphan","type":"null","name":"Orphan","from":2,"dur":2,"parent":"outside","group":"absent","matteSource":"missing","p":{}}
    ]}}}"##).unwrap()
}
fn obj(v: &mut Value) -> &mut pm_model::Map {
    v.as_object_mut().unwrap()
}
fn host(v: &mut Value) -> &mut Value {
    &mut v
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap()[2]
}
fn source(v: &mut Value) -> &mut Vec<Value> {
    v.as_object_mut()
        .unwrap()
        .get_mut("comps")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("sub")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap()
}
fn run(s: &mut DocumentStore) -> Result<(), String> {
    s.execute(
        Command::Comp(CompCommand::ReleasePrecomp {
            layer_id: "pre".into(),
        }),
        Origin::Interface,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}
fn all(s: &DocumentStore) -> Vec<Value> {
    s.root()
        .unwrap()
        .get("comps")
        .unwrap()
        .get("root")
        .unwrap()
        .get("layers")
        .unwrap()
        .to_value()
        .as_array()
        .unwrap()
        .clone()
}
fn named<'a>(list: &'a [Value], name: &str) -> &'a Value {
    list.iter()
        .find(|l| l.get("name").and_then(Value::as_str) == Some(name))
        .unwrap()
}
fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or_else(|| panic!("missing {k} in {v:?}"))
}
fn number(v: &Value, k: &str) -> f64 {
    field(v, k)
        .as_f64()
        .unwrap_or_else(|| panic!("not numeric {k}: {v:?}"))
}

#[test]
fn e12_release_keeps_carrier_rig_channels_and_original_comp_and_is_one_undo() {
    let value = fixture();
    let original = host(&mut value.clone()).clone();
    let mut s = DocumentStore::new_untitled(value);
    let before = s.root().unwrap().to_value();
    run(&mut s).unwrap();
    let released = all(&s);
    let carrier = named(&released, "Nested picture");
    assert_eq!(field(carrier, "type").as_str(), Some("group"));
    assert_eq!(field(carrier, "collapsed"), &Value::Bool(false));
    for key in [
        "id",
        "from",
        "dur",
        "on",
        "shy",
        "solo",
        "color",
        "blend",
        "mblur",
        "parent",
        "group",
        "p",
        "fx",
        "masks",
        "matteSource",
        "matteMode",
        "transitionIn",
        "transitionOut",
        "track",
    ] {
        assert_eq!(carrier.get(key), original.get(key), "carrier {key}");
    }
    let after = s.root().unwrap().to_value();
    assert_eq!(
        after.get("comps").unwrap().get("sub"),
        before.get("comps").unwrap().get("sub")
    );
    assert_eq!(
        named(&released, "Other user").get("d"),
        Some(&parse(r#"{"comp":"sub"}"#).unwrap())
    );
    s.undo().unwrap();
    assert_eq!(s.root().unwrap().to_value(), before);
    s.redo().unwrap();
    assert_eq!(s.root().unwrap().to_value(), after);
}
#[test]
fn e12_release_remaps_internal_links_renews_child_keys_and_preserves_external_inputs() {
    let mut s = DocumentStore::new_untitled(fixture());
    run(&mut s).unwrap();
    let list = all(&s);
    let clip = named(&list, "Clip");
    let driver = named(&list, "Driver");
    let mask = named(&list, "Mask");
    let group = named(&list, "Inner group");
    let orphan = named(&list, "Orphan");
    assert_ne!(field(clip, "id").as_str(), Some("clip"));
    assert_eq!(clip.get("parent"), driver.get("id"));
    assert_eq!(clip.get("group"), group.get("id"));
    assert_eq!(clip.get("matteSource"), mask.get("id"));
    assert_eq!(group.get("group").and_then(Value::as_str), Some("pre"));
    assert_eq!(orphan.get("group").and_then(Value::as_str), Some("pre"));
    assert_eq!(orphan.get("parent"), Some(&Value::Null));
    assert_eq!(orphan.get("matteSource"), Some(&Value::Null));
    let fx = &field(clip, "fx").as_array().unwrap()[0];
    let inputs = field(fx, "inputs");
    assert_eq!(field(inputs, "map").get("layerId"), mask.get("id"));
    assert_eq!(
        field(inputs, "map").get("space").and_then(Value::as_str),
        Some("source")
    );
    assert_eq!(
        field(inputs, "external")
            .get("layerId")
            .and_then(Value::as_str),
        Some("external")
    );
    assert_ne!(
        field(field(clip, "p"), "position.x")
            .get("kf")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .get("i")
            .and_then(Value::as_str),
        Some("child-key")
    );
    assert_ne!(
        field(field(fx, "p"), "gain")
            .get("kf")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .get("i")
            .and_then(Value::as_str),
        Some("effect-key")
    );
}
#[test]
fn e12_release_offsets_clips_source_trim_captions_and_local_keys_without_removing_hidden_links() {
    let mut s = DocumentStore::new_untitled(fixture());
    run(&mut s).unwrap();
    let list = all(&s);
    let clip = named(&list, "Clip");
    assert_eq!(number(clip, "from"), 3.);
    assert_eq!(number(clip, "dur"), 3.);
    assert_eq!(
        field(field(clip, "d"), "trim")
            .get("v")
            .and_then(Value::as_f64),
        Some(2.5)
    );
    assert_eq!(
        field(field(clip, "p"), "position.x")
            .get("kf")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .get("t")
            .and_then(Value::as_f64),
        Some(1.)
    );
    let hidden = named(&list, "Outside span");
    assert_eq!(field(hidden, "on"), &Value::Bool(false));
    assert_eq!(hidden.get("parent"), clip.get("id"));
    assert_eq!(number(hidden, "from"), 9.);
    let cues = field(field(named(&list, "Captions"), "d"), "cues")
        .as_array()
        .unwrap();
    assert_eq!(number(&cues[0], "start"), 0.);
    assert_eq!(number(&cues[0], "end"), 1.);
    assert_eq!(
        number(&field(&cues[0], "words").as_array().unwrap()[0], "start"),
        0.
    );
    assert_eq!(number(named(&list, "Inner group"), "from"), 2.);
    assert_eq!(number(named(&list, "Inner group"), "dur"), 8.);
}
#[test]
fn e12_release_guards_are_atomic_and_match_main_admission() {
    for case in [
        "wrong-type",
        "missing-comp",
        "locked",
        "ancestor-locked",
        "3d",
        "source",
        "remap",
        "speed",
        "animated-speed",
        "expression-speed",
        "camera",
        "light",
    ] {
        let mut v = fixture();
        match case {
            "wrong-type" => {
                obj(host(&mut v)).insert("type", Value::from("solid"));
            }
            "missing-comp" => {
                obj(host(&mut v)).insert("d", parse(r#"{"comp":"missing"}"#).unwrap());
            }
            "locked" => {
                obj(host(&mut v)).insert("lock", Value::Bool(true));
            }
            "ancestor-locked" => {
                let l = obj(&mut v)
                    .get_mut("layers")
                    .unwrap()
                    .as_array_mut()
                    .unwrap();
                obj(&mut l[0]).insert("lock", Value::Bool(true));
            }
            "3d" => {
                obj(host(&mut v)).insert("threeD", Value::Bool(true));
            }
            "source" => {
                obj(host(&mut v)).insert("effectSpace", Value::from("source"));
            }
            "camera" | "light" => {
                obj(&mut source(&mut v)[0]).insert("type", Value::from("extension"));
                obj(&mut source(&mut v)[0]).insert("d", {
                    let role = if case == "camera" {
                        pm_scene3d::schema::Role::Camera
                    } else {
                        pm_scene3d::schema::Role::Light
                    };
                    let input = if case == "camera" {
                        serde_json::json!({})
                    } else {
                        serde_json::json!({"light":{"type":"point","p":{},"castShadow":true}})
                    };
                    let data = pm_scene3d::schema::layer_data(role, input).unwrap();
                    pm_scene3d::to_model(
                        &serde_json::json!({"definition":role.definition(),"data":data}),
                    )
                });
            }
            _ => {
                let d = obj(host(&mut v)).get_mut("d").unwrap();
                let (k, val) = match case {
                    "remap" => ("timeRemap", r#"{"v":true,"kf":[]}"#),
                    "speed" => ("speed", "2"),
                    "animated-speed" => ("speed", r#"{"v":1,"kf":[{"t":0,"v":1}]}"#),
                    _ => ("speed", r#"{"v":1,"kf":[],"expr":"value"}"#),
                };
                obj(d).insert(k, parse(val).unwrap());
            }
        }
        let mut s = DocumentStore::new_untitled(v);
        let before = s.root().unwrap().to_value();
        let rev = s.revision();
        if matches!(case, "camera" | "light") {
            let saved = before
                .get("comps")
                .unwrap()
                .get("sub")
                .unwrap()
                .get("layers")
                .unwrap()
                .as_array()
                .unwrap();
            assert_eq!(
                saved[0]
                    .get("d")
                    .and_then(|d| d.get("definition"))
                    .and_then(Value::as_str),
                Some(format!("powermove.3d.{case}").as_str()),
                "fixture must keep its 3D scene definition"
            );
        }
        let error = run(&mut s).expect_err(case);
        let expected = match case {
            "wrong-type" => "Select a pre-comp layer",
            "missing-comp" => "composition is missing",
            "locked" | "ancestor-locked" => "Unlock the pre-comp",
            "3d" => "Turn off 3D",
            "source" => "Set Effect Space to Composition",
            "camera" | "light" => "own camera or lights",
            _ => "Remove time remapping and speed changes",
        };
        assert!(error.contains(expected), "{case}: {error}");
        assert_eq!(s.revision(), rev, "{case}");
        assert_eq!(s.root().unwrap().to_value(), before, "{case}");
    }
}
