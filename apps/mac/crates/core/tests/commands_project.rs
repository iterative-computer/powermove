//! Effects, transitions, compositions, markers, content, masks and paths,
//! text, media, library and nudges against the old app's semantics.

use pm_mac_core::commands::comps::{CompPatch, NewComp};
use pm_mac_core::commands::effects::{
    AddEffect, Edge, EffectDefinition, EffectPatch, ParamDefinition, TransitionDefinition,
    TransitionSpec,
};
use pm_mac_core::commands::masks::{MaskPatch, MaskShape};
use pm_mac_core::commands::properties::SetProperty;
use pm_mac_core::{
    Command, CommandError, CompCommand, ContentCommand, DocumentStore, EffectCommand,
    LibraryCommand, MarkerCommand, MaskCommand, MediaCommand, Origin, PropertyCommand, TextCommand,
    TransactionError, TransformCommand,
};
use pm_model::Map;
use pm_model::json::{Value, parse};

const PROJECT: &str = r##"{
  "id": "P", "name": "Project", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 10, "bg": "#000000", "work": [0, 10],
  "assets": {"a1": {"id": "a1", "kind": "video", "name": "clip.mov", "dur": 8},
             "a2": {"id": "a2", "kind": "video", "name": "other.mov", "dur": 4}},
  "markers": [{"id": "m1", "t": 5, "name": "Five"}],
  "layers": [
    {"id": "box", "type": "shape", "name": "Box", "from": 0, "dur": 10, "on": true,
     "d": {"shape": "rect", "color": "#ff0000", "w": 40, "h": 40, "radius": 0,
           "size": 3},
     "fx": [{"id": "fx1", "type": "blur", "version": 1, "on": true,
             "p": {"amount": {"v": 4, "kf": [{"t": 1, "v": 4, "i": "kb"}], "expr": null}}}],
     "masks": [],
     "p": {"position.x": {"v": 50, "kf": [], "expr": null}, "position.y": {"v": 20, "kf": [], "expr": null}}},
    {"id": "title", "type": "text", "name": "Title", "from": 0, "dur": 10,
     "d": {"text": "Hello", "boxWidth": {"v": 0, "kf": [], "expr": null},
           "boxHeight": {"v": 0, "kf": [], "expr": null}, "size": 64},
     "p": {"position.x": {"v": 0, "kf": [{"t": 0, "v": 0, "i": "kp1"}, {"t": 2, "v": 100, "i": "kp2"}], "expr": null},
           "position.y": {"v": 0, "kf": [], "expr": null}}},
    {"id": "clip", "type": "video", "name": "Clip", "from": 0, "dur": 4,
     "d": {"asset": "a1", "fit": "cover", "trim": 0, "speed": 1, "w": 10, "h": 10}, "p": {}},
    {"id": "sound", "type": "audio", "name": "Sound", "from": 0, "dur": 4,
     "d": {"asset": "a1", "trim": 0, "gain": 1, "fadeIn": 0, "fadeOut": 0}, "p": {}},
    {"id": "pre", "type": "precomp", "name": "Inner", "from": 0, "dur": 10,
     "d": {"comp": "C2", "w": 10, "h": 10}, "p": {}}
  ],
  "comps": {"C2": {"id": "C2", "name": "Inner", "w": 10, "h": 10, "fps": 30, "dur": 2,
    "layers": [{"id": "inner", "type": "shape", "name": "Inner shape", "from": 0, "dur": 2, "p": {}}]}}
}"##;

fn store() -> DocumentStore {
    DocumentStore::new_untitled(parse(PROJECT).unwrap())
}

fn layer(store: &DocumentStore, id: &str) -> Value {
    let root = store.root().unwrap();
    let loc = pm_mac_core::model::find_layer(root, id).unwrap_or_else(|| panic!("no {id}"));
    pm_mac_core::model::layer_at(root, &loc).unwrap().to_value()
}

fn root(store: &DocumentStore) -> Value {
    store.root().unwrap().to_value()
}

fn get<'v>(v: &'v Value, path: &[&str]) -> &'v Value {
    let mut cur = v;
    for p in path {
        cur = match p.parse::<usize>() {
            Ok(i) => &cur.as_array().unwrap()[i],
            Err(_) => cur.get(p).unwrap_or_else(|| panic!("no {p} in {cur:?}")),
        };
    }
    cur
}

fn num(v: &Value, path: &[&str]) -> f64 {
    get(v, path).as_f64().unwrap()
}

fn run(store: &mut DocumentStore, c: impl Into<Command>) {
    store.execute(c.into(), Origin::Interface).unwrap();
}

fn refused(store: &mut DocumentStore, c: impl Into<Command>) -> CommandError {
    let before = store.revision();
    match store.execute(c.into(), Origin::Interface) {
        Err(TransactionError::Command(e)) => {
            assert_eq!(store.revision(), before);
            e
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn glow() -> EffectDefinition {
    EffectDefinition {
        effect_type: "glow".into(),
        version: 2,
        label: "Glow".into(),
        params: vec![
            ParamDefinition {
                key: "radius".into(),
                kind: "number".into(),
                default: Value::Number(10.0),
                min: Some(0.0),
                max: None,
                reference: false,
            },
            ParamDefinition {
                key: "source".into(),
                kind: "layer".into(),
                default: Value::Null,
                min: None,
                max: None,
                reference: true,
            },
        ],
        source_processing: true,
        backdrop: false,
    }
}

fn add_glow(id: &str) -> AddEffect {
    AddEffect {
        layer_id: "box".into(),
        effect_id: id.into(),
        definition: glow(),
        parameters: Map::new(),
        inputs: None,
        open: true,
        enabled: true,
        enabled_animation: None,
        index: None,
    }
}

// ── effects ─────────────────────────────────────────────────────────────────

#[test]
fn effects_are_built_from_their_definition_and_edited() {
    let mut s = store();
    let mut c = add_glow("fx2");
    c.parameters.insert("radius", Value::Number(25.0));
    c.parameters.insert("unknown", Value::Number(1.0));
    run(&mut s, EffectCommand::Add(Box::new(c)));
    let fx = get(&layer(&s, "box"), &["fx", "1"]).clone();
    assert_eq!(get(&fx, &["type"]).as_str(), Some("glow"));
    assert_eq!(num(&fx, &["version"]), 2.0);
    assert_eq!(num(&fx, &["p", "radius", "v"]), 25.0);
    assert!(fx.get("p").unwrap().get("unknown").is_none());
    assert!(
        get(&fx, &["inputs", "source", "layerId"]).is_null(),
        "an empty reference"
    );

    // Paste of an animated channel gets fresh key ids.
    let mut paste = add_glow("fx3");
    paste.parameters.insert(
        "radius",
        parse(r#"{"v": 3, "kf": [{"t": 0, "v": 3, "i": "kb"}], "expr": "value"}"#).unwrap(),
    );
    run(&mut s, EffectCommand::Add(Box::new(paste)));
    let k = get(
        &layer(&s, "box"),
        &["fx", "2", "p", "radius", "kf", "0", "i"],
    )
    .as_str()
    .unwrap()
    .to_owned();
    assert_ne!(k, "kb");

    run(
        &mut s,
        EffectCommand::Set {
            layer_id: "box".into(),
            effect: "fx3".into(),
            patch: EffectPatch {
                index: Some(0),
                enabled: Some(false),
                ..Default::default()
            },
        },
    );
    let l = layer(&s, "box");
    assert_eq!(get(&l, &["fx", "0", "id"]).as_str(), Some("fx3"));
    assert_eq!(get(&l, &["fx", "0", "on"]), &Value::Bool(false));

    run(
        &mut s,
        EffectCommand::Duplicate {
            layer_id: "box".into(),
            effect_ids: vec!["fx1".into()],
            new_ids: vec!["fx1b".into()],
        },
    );
    let l = layer(&s, "box");
    let order: Vec<&str> = l
        .get("fx")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.get("id").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(order, ["fx3", "fx1", "fx1b", "fx2"]);
    assert_ne!(
        get(&l, &["fx", "2", "p", "amount", "kf", "0", "i"]).as_str(),
        Some("kb")
    );

    run(
        &mut s,
        EffectCommand::Remove {
            layer_id: "box".into(),
            effect: "glow".into(),
        },
    );
    assert_eq!(
        layer(&s, "box")
            .get("fx")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2,
        "every glow goes"
    );
    let e = refused(
        &mut s,
        EffectCommand::Remove {
            layer_id: "box".into(),
            effect: "nope".into(),
        },
    );
    assert_eq!(e.to_string(), "Effect not found: nope");
    let mut audio = add_glow("fx9");
    audio.layer_id = "sound".into();
    let e = refused(&mut s, EffectCommand::Add(Box::new(audio)));
    assert_eq!(e.to_string(), "Audio layers do not support visual effects");
    let mut bad = add_glow("fx8");
    bad.inputs = Some(parse(r#"{"other": {"layerId": "clip"}}"#).unwrap());
    let e = refused(&mut s, EffectCommand::Add(Box::new(bad)));
    assert_eq!(e.to_string(), "Unknown layer input: other");
}

#[test]
fn transitions_keep_channels_of_the_same_type() {
    let mut s = store();
    let def = TransitionDefinition {
        transition_type: "fade".into(),
        params: vec![ParamDefinition {
            key: "softness".into(),
            kind: "number".into(),
            default: Value::Number(0.5),
            min: None,
            max: None,
            reference: false,
        }],
    };
    let spec = |dur: f64, params: Map| TransitionSpec {
        definition: def.clone(),
        duration: Some(dur),
        params,
    };
    run(
        &mut s,
        EffectCommand::SetTransition {
            layer_id: "box".into(),
            edge: Edge::In,
            transition: Some(Box::new(spec(1.0, Map::new()))),
        },
    );
    assert_eq!(
        num(&layer(&s, "box"), &["transitionIn", "p", "softness", "v"]),
        0.5
    );
    // Key the parameter, then change only the duration: the key survives.
    run(
        &mut s,
        PropertyCommand::Set(SetProperty {
            mode: pm_mac_core::commands::properties::KeyMode::Keyframe,
            ..SetProperty::new("box", "transitionIn.p.softness", 0.8, 0.0)
        }),
    );
    run(
        &mut s,
        EffectCommand::SetTransition {
            layer_id: "box".into(),
            edge: Edge::In,
            transition: Some(Box::new(spec(2.0, Map::new()))),
        },
    );
    let t = get(&layer(&s, "box"), &["transitionIn"]).clone();
    assert_eq!(num(&t, &["dur"]), 2.0);
    assert_eq!(
        get(&t, &["p", "softness", "kf"]).as_array().unwrap().len(),
        1
    );
    let e = refused(
        &mut s,
        EffectCommand::SetTransition {
            layer_id: "box".into(),
            edge: Edge::Out,
            transition: Some(Box::new(spec(0.001, Map::new()))),
        },
    );
    assert_eq!(
        e.to_string(),
        "Transition duration must be between 0.02 and 600 seconds"
    );
    let mut p = Map::new();
    p.insert("softness", Value::String("x".into()));
    let e = refused(
        &mut s,
        EffectCommand::SetTransition {
            layer_id: "box".into(),
            edge: Edge::Out,
            transition: Some(Box::new(spec(1.0, p))),
        },
    );
    assert_eq!(
        e.to_string(),
        "Transition parameter “softness” must be a finite number"
    );
    run(
        &mut s,
        EffectCommand::SetTransition {
            layer_id: "box".into(),
            edge: Edge::In,
            transition: None,
        },
    );
    assert!(get(&layer(&s, "box"), &["transitionIn"]).is_null());
}

// ── compositions ────────────────────────────────────────────────────────────

#[test]
fn set_composition_clamps_like_the_old_command() {
    let mut s = store();
    run(
        &mut s,
        CompCommand::Set {
            patch: Box::new(CompPatch {
                name: Some(" Renamed ".into()),
                width: Some(10.4),
                fps: Some(500.0),
                duration: Some(4.0),
                background: Some("#112233".into()),
                ..Default::default()
            }),
        },
    );
    let r = root(&s);
    let c = get(&r, &["comps", "C1"]);
    assert_eq!(
        get(&r, &["name"]).as_str(),
        Some("Renamed"),
        "the project name"
    );
    assert_eq!(num(c, &["w"]), 16.0);
    assert_eq!(num(c, &["fps"]), 240.0);
    assert_eq!(get(c, &["work"]), &parse("[0, 4]").unwrap());
    assert_eq!(
        get(c, &["backgroundFill", "stops", "0", "color"]).as_str(),
        Some("#112233")
    );
    let e = refused(
        &mut s,
        CompCommand::Set {
            patch: Box::new(CompPatch {
                color_depth: Some(12),
                ..Default::default()
            }),
        },
    );
    assert_eq!(e.to_string(), "Color depth must be 8, 16, or 32");
}

#[test]
fn work_area_edges_keep_a_frame() {
    let mut s = store();
    run(
        &mut s,
        CompCommand::SetWorkArea {
            comp: None,
            start: Some(3.0),
            end: None,
        },
    );
    run(
        &mut s,
        CompCommand::SetWorkArea {
            comp: None,
            start: None,
            end: Some(1.0),
        },
    );
    let w = get(&root(&s), &["comps", "C1", "work"]).clone();
    assert_eq!(num(&w, &["0"]), 3.0);
    assert!((num(&w, &["1"]) - (3.0 + 1.0 / 30.0)).abs() < 1e-12);
    run(&mut s, CompCommand::ResetWorkArea { comp: None });
    assert_eq!(
        get(&root(&s), &["comps", "C1", "work"]),
        &parse("[0, 10]").unwrap()
    );
}

#[test]
fn create_duplicate_rename_and_delete_compositions() {
    let mut s = store();
    run(
        &mut s,
        CompCommand::Create {
            comp_id: "C3".into(),
            settings: NewComp::default(),
        },
    );
    let c3 = get(&root(&s), &["comps", "C3"]).clone();
    assert_eq!(get(&c3, &["name"]).as_str(), Some("Comp 1"));
    assert_eq!((num(&c3, &["w"]), num(&c3, &["dur"])), (200.0, 10.0));
    run(
        &mut s,
        CompCommand::Duplicate {
            comp_id: "C2".into(),
            new_id: "C4".into(),
        },
    );
    let c4 = get(&root(&s), &["comps", "C4"]).clone();
    assert_eq!(get(&c4, &["name"]).as_str(), Some("Inner 2"));
    assert_ne!(
        get(&c4, &["layers", "0", "id"]).as_str(),
        Some("inner"),
        "fresh layer ids"
    );
    assert_eq!(
        get(&c4, &["layers", "0", "name"]).as_str(),
        Some("Inner shape")
    );
    // Renaming keeps names unique and renames pre-comp layers showing it.
    run(
        &mut s,
        CompCommand::Settings {
            comp_id: "C2".into(),
            settings: NewComp {
                name: Some("Inner 2".into()),
                width: Some(20.0),
                ..Default::default()
            },
        },
    );
    assert_eq!(
        get(&root(&s), &["comps", "C2", "name"]).as_str(),
        Some("Inner 3")
    );
    let pre = layer(&s, "pre");
    assert_eq!(get(&pre, &["name"]).as_str(), Some("Inner 3"));
    assert_eq!(num(&pre, &["d", "w"]), 20.0);
    // Deleting a composition removes the pre-comp layers that use it.
    run(
        &mut s,
        CompCommand::Delete {
            comp_id: "C2".into(),
            fallback: None,
        },
    );
    assert!(root(&s).get("comps").unwrap().get("C2").is_none());
    assert!(!s.layers().iter().any(|l| l.id == "pre"));
    s.undo().unwrap();
    assert!(s.layers().iter().any(|l| l.id == "pre"));
}

#[test]
fn scene_parameters_and_markers() {
    let mut s = store();
    run(
        &mut s,
        CompCommand::SetSceneParameter {
            name: " speed ".into(),
            label: None,
            control: None,
            value: Value::Number(2.0),
            min: Some(Value::Number(0.0)),
            max: None,
            options: None,
        },
    );
    let p = get(&root(&s), &["params", "speed"]).clone();
    assert_eq!(get(&p, &["label"]).as_str(), Some("speed"));
    assert_eq!(get(&p, &["control"]).as_str(), Some("num"));
    assert_eq!(num(&p, &["value"]), 2.0);

    run(
        &mut s,
        MarkerCommand::Add {
            comp: None,
            id: "m2".into(),
            time: 20.0,
            name: None,
        },
    );
    run(
        &mut s,
        MarkerCommand::Add {
            comp: None,
            id: "m3".into(),
            time: 1.0,
            name: Some("One".into()),
        },
    );
    let ms = get(&root(&s), &["comps", "C1", "markers"]).clone();
    let names: Vec<&str> = ms
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.get("name").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["One", "Five", "M2"],
        "sorted; clamped to the end; numbered"
    );
    assert_eq!(num(&ms, &["2", "t"]), 10.0);
    run(
        &mut s,
        MarkerCommand::Remove {
            comp: None,
            index: 0,
        },
    );
    run(
        &mut s,
        MarkerCommand::Move {
            comp: None,
            id: "m2".into(),
            time: 2.0,
        },
    );
    let ms = get(&root(&s), &["comps", "C1", "markers"]).clone();
    assert_eq!(get(&ms, &["0", "id"]).as_str(), Some("m2"));
    let e = refused(
        &mut s,
        MarkerCommand::Remove {
            comp: None,
            index: 9,
        },
    );
    assert_eq!(e.to_string(), "Marker not found");
}

// ── content, text, media ────────────────────────────────────────────────────

#[test]
fn set_content_merges_keys_animated_fields_and_guards_audio() {
    let mut s = store();
    let patch = |pairs: &[(&str, Value)]| {
        let mut m = Map::new();
        for (k, v) in pairs {
            m.insert(*k, v.clone());
        }
        m
    };
    run(
        &mut s,
        ContentCommand::Set {
            layer_id: "box".into(),
            patch: patch(&[
                ("w", Value::Number(99.0)),
                ("color", Value::String("#00ff00".into())),
            ]),
            time: 0.0,
        },
    );
    let d = get(&layer(&s, "box"), &["d"]).clone();
    assert_eq!(
        (num(&d, &["w"]), get(&d, &["color"]).as_str()),
        (99.0, Some("#00ff00"))
    );
    // An animated content channel is keyed instead.
    run(
        &mut s,
        PropertyCommand::Set(SetProperty {
            mode: pm_mac_core::commands::properties::KeyMode::Keyframe,
            ..SetProperty::new("box", "c.h", 40.0, 0.0)
        }),
    );
    run(
        &mut s,
        ContentCommand::Set {
            layer_id: "box".into(),
            patch: patch(&[("h", Value::Number(80.0))]),
            time: 1.0,
        },
    );
    assert_eq!(
        get(&layer(&s, "box"), &["d", "h", "kf"])
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Audio: only its fields, clamped.
    run(
        &mut s,
        ContentCommand::Set {
            layer_id: "sound".into(),
            patch: patch(&[("gain", Value::Number(9.0))]),
            time: 0.0,
        },
    );
    assert_eq!(num(&layer(&s, "sound"), &["d", "gain"]), 4.0);
    let e = refused(
        &mut s,
        ContentCommand::Set {
            layer_id: "sound".into(),
            patch: patch(&[("w", Value::Number(1.0))]),
            time: 0.0,
        },
    );
    assert_eq!(e.to_string(), "Audio content field “w” is not editable");
    let e = refused(
        &mut s,
        ContentCommand::Set {
            layer_id: "box".into(),
            patch: patch(&[("__proto__", Value::Null)]),
            time: 0.0,
        },
    );
    assert_eq!(e.to_string(), "Unsafe content patch key: __proto__");
    // Video audio gain clamps; null clears it.
    run(
        &mut s,
        ContentCommand::Set {
            layer_id: "clip".into(),
            patch: patch(&[("audioGain", Value::Number(7.0))]),
            time: 0.0,
        },
    );
    assert_eq!(num(&layer(&s, "clip"), &["d", "audioGain"]), 4.0);
    run(
        &mut s,
        ContentCommand::Set {
            layer_id: "clip".into(),
            patch: patch(&[("audioGain", Value::Null)]),
            time: 0.0,
        },
    );
    assert!(
        layer(&s, "clip")
            .get("d")
            .unwrap()
            .get("audioGain")
            .is_none()
    );
}

#[test]
fn text_and_media() {
    let mut s = store();
    run(
        &mut s,
        TextCommand::SetText {
            layer_id: "title".into(),
            text: "Hi\nthere".into(),
            time: 0.0,
        },
    );
    assert_eq!(
        get(&layer(&s, "title"), &["d", "text"]).as_str(),
        Some("Hi\nthere")
    );
    run(
        &mut s,
        TextCommand::SetBox {
            layer_id: "title".into(),
            width: Some(300.0),
            height: None,
            time: 0.0,
        },
    );
    let t = layer(&s, "title");
    assert_eq!(num(&t, &["d", "boxWidth", "v"]), 300.0);
    assert_eq!(num(&t, &["d", "size"]), 64.0, "Size is not rewritten");
    let e = refused(
        &mut s,
        TextCommand::SetText {
            layer_id: "box".into(),
            text: "x".into(),
            time: 0.0,
        },
    );
    assert_eq!(e.to_string(), "Not a text layer");

    run(
        &mut s,
        MediaCommand::Replace {
            layer_id: "clip".into(),
            asset_id: "a2".into(),
        },
    );
    assert_eq!(
        get(&layer(&s, "clip"), &["d", "asset"]).as_str(),
        Some("a2")
    );
    run(
        &mut s,
        MediaCommand::RenameAsset {
            asset_id: "a2".into(),
            name: " B-roll ".into(),
        },
    );
    assert_eq!(
        get(&root(&s), &["assets", "a2", "name"]).as_str(),
        Some("B-roll")
    );
    // Removing media removes the layers that show it.
    run(
        &mut s,
        MediaCommand::RemoveAsset {
            asset_id: "a1".into(),
        },
    );
    assert!(!s.layers().iter().any(|l| l.id == "sound"));
    assert!(root(&s).get("assets").unwrap().get("a1").is_none());
}

#[test]
fn masks_and_pen_paths() {
    let mut s = store();
    run(
        &mut s,
        MaskCommand::Add {
            layer_id: "box".into(),
            mask_id: "K1".into(),
            shape: MaskShape::Ellipse,
        },
    );
    let m = get(&layer(&s, "box"), &["masks", "0"]).clone();
    assert_eq!(get(&m, &["shape"]).as_str(), Some("ellipse"));
    assert_eq!(num(&m, &["p", "w", "v"]), 50.0, "half the shorter side");
    assert_eq!(num(&m, &["p", "feather", "v"]), 24.0);
    run(
        &mut s,
        MaskCommand::Set {
            layer_id: "box".into(),
            mask_id: "K1".into(),
            patch: MaskPatch {
                mode: Some("subtract".into()),
                ..Default::default()
            },
        },
    );
    assert_eq!(
        get(&layer(&s, "box"), &["masks", "0", "mode"]).as_str(),
        Some("subtract")
    );

    // Pen on a shape layer: a path, three vertices, a drag, close.
    s.transact(
        "Draw path vertex",
        Origin::Interface,
        &[
            MaskCommand::AddPath {
                layer_id: "box".into(),
                path_id: "path1".into(),
                mask_id: None,
            }
            .into(),
            MaskCommand::AddVertex {
                layer_id: "box".into(),
                path_id: "path1".into(),
                vertex_id: "v1".into(),
                x: 0.0,
                y: 0.0,
            }
            .into(),
        ],
    )
    .unwrap();
    run(
        &mut s,
        MaskCommand::AddVertex {
            layer_id: "box".into(),
            path_id: "path1".into(),
            vertex_id: "v2".into(),
            x: 10.0,
            y: 0.0,
        },
    );
    run(
        &mut s,
        PropertyCommand::Set(SetProperty::new("box", "g.path1.v.v2.y", 5.0, 0.0)),
    );
    run(
        &mut s,
        MaskCommand::ClosePath {
            layer_id: "box".into(),
            path_id: "path1".into(),
        },
    );
    let path = get(&layer(&s, "box"), &["d", "paths", "0"]).clone();
    assert_eq!(num(&path, &["vertices", "1", "p", "y", "v"]), 5.0);
    assert_eq!(get(&path, &["p", "closed", "v"]), &Value::Bool(true));
    run(
        &mut s,
        MaskCommand::DeleteVertex {
            layer_id: "box".into(),
            path_id: "path1".into(),
            vertex_id: "v1".into(),
        },
    );
    assert_eq!(
        get(&layer(&s, "box"), &["d", "paths", "0", "vertices"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Elsewhere the pen makes a mask holding the path.
    run(
        &mut s,
        MaskCommand::AddPath {
            layer_id: "title".into(),
            path_id: "path2".into(),
            mask_id: Some("K2".into()),
        },
    );
    let m = get(&layer(&s, "title"), &["masks", "0"]).clone();
    assert_eq!(get(&m, &["path", "id"]).as_str(), Some("path2"));
    assert_eq!(num(&m, &["p", "feather", "v"]), 0.0);
}

#[test]
fn nudges_key_animated_positions_in_parent_space() {
    let mut s = store();
    run(
        &mut s,
        TransformCommand::Nudge {
            layer_ids: vec!["box".into(), "title".into()],
            dx: 10.0,
            dy: -1.0,
            time: 1.0,
        },
    );
    let b = layer(&s, "box");
    assert_eq!(
        (
            num(&b, &["p", "position.x", "v"]),
            num(&b, &["p", "position.y", "v"])
        ),
        (60.0, 19.0)
    );
    // Title's x is keyed: 50 at 1 s (linear 0 → 100 over 2 s) becomes 60.
    let k = get(&layer(&s, "title"), &["p", "position.x", "kf"])
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(k.len(), 3);
    assert_eq!((num(&k[1], &["t"]), num(&k[1], &["v"])), (1.0, 60.0));
}

#[test]
fn library_sections() {
    let mut s = store();
    run(
        &mut s,
        LibraryCommand::CreateSection {
            section: parse(r#"{"id": "S1", "name": "Intro", "layers": []}"#).unwrap(),
        },
    );
    let e = refused(
        &mut s,
        LibraryCommand::CreateSection {
            section: parse(r#"{"id": "S1"}"#).unwrap(),
        },
    );
    assert_eq!(e.to_string(), "Section id already exists");
    run(
        &mut s,
        LibraryCommand::UpdateSection {
            section_id: "S1".into(),
            layers: vec![parse(r#"{"id": "x", "type": "text"}"#).unwrap()],
            thumb: None,
            at: 1000.0,
            version: None,
        },
    );
    let sec = get(&root(&s), &["library", "sections", "0"]).clone();
    assert_eq!(get(&sec, &["tags"]), &parse(r#"["text"]"#).unwrap());
    assert_eq!(get(&sec, &["versions"]).as_array().unwrap().len(), 1);
}
