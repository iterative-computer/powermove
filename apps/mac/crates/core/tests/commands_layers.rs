//! Layer, timing and group commands against the old app's semantics
//! (`editing.ts` `setLayer`/`addLayer`/`deleteLayers`/`reorderLayer`,
//! `model.ts`, `shortcuts.ts` paste/duplicate/order/split, the timeline's
//! trims, `layer-groups.ts`).

use pm_mac_core::commands::keyframes::KeyframeCommand;
use pm_mac_core::commands::layers::{AddLayer, LayerPatch, ParentChange, Restack};
use pm_mac_core::{
    Command, CommandError, DocumentStore, GroupCommand, LayerCommand, Origin, TimingCommand,
    TransactionError,
};
use pm_model::Map;
use pm_model::json::{Value, parse};

const PROJECT: &str = r##"{
  "id": "P", "name": "Test", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 10, "bg": "#000000",
  "assets": {"a1": {"id": "a1", "kind": "video", "name": "clip.mov", "dur": 8}},
  "layers": [
    {"id": "top", "type": "shape", "name": "Top", "from": 0, "dur": 10, "on": true,
     "d": {"shape": "rect", "color": "#ffffff", "w": 20, "h": 20, "radius": 0},
     "p": {"position.x": {"v": 10, "kf": [], "expr": null}, "position.y": {"v": 0, "kf": [], "expr": null},
           "scale.x": {"v": 100, "kf": [], "expr": null}, "scale.y": {"v": 100, "kf": [], "expr": null},
           "rotation": {"v": 0, "kf": [], "expr": null}, "anchor.x": {"v": 0, "kf": [], "expr": null},
           "anchor.y": {"v": 0, "kf": [], "expr": null},
           "opacity": {"v": 100, "kf": [{"t": 2, "v": 0, "i": "kt"}], "expr": null}}},
    {"id": "box", "type": "shape", "name": "Box", "from": 1, "dur": 4, "on": true,
     "d": {"shape": "rect", "color": "#ffffff", "w": 40, "h": 40, "radius": 0},
     "p": {"position.x": {"v": 50, "kf": [], "expr": null}, "position.y": {"v": 20, "kf": [], "expr": null},
           "scale.x": {"v": 100, "kf": [], "expr": null}, "scale.y": {"v": 100, "kf": [], "expr": null},
           "rotation": {"v": 0, "kf": [], "expr": null}, "anchor.x": {"v": 0, "kf": [], "expr": null},
           "anchor.y": {"v": 0, "kf": [], "expr": null}, "opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "clip", "type": "video", "name": "Clip", "from": 2, "dur": 4, "on": true,
     "d": {"asset": "a1", "fit": "cover", "trim": 1, "speed": 1, "w": 10, "h": 10},
     "p": {"opacity": {"v": 100, "kf": [{"t": 1, "v": 50, "i": "kc1"}, {"t": 3, "v": 80, "i": "kc2"}], "expr": null}}},
    {"id": "sound", "type": "audio", "name": "Sound", "from": 0, "dur": 6,
     "d": {"asset": null, "trim": 0, "gain": 1, "fadeIn": 0.5, "fadeOut": 0.5}, "p": {}},
    {"id": "locked", "type": "shape", "name": "Locked", "lock": true, "from": 0, "dur": 10, "p": {}},
    {"id": "bottom", "type": "solid", "name": "Bottom", "from": 0, "dur": 10, "p": {}}
  ]
}"##;

/// Every transform channel `PM.mkLayer` creates, filled in where the
/// fixture leaves it out (real layers always have them).
fn store() -> DocumentStore {
    let mut project = parse(PROJECT).unwrap();
    let defaults = [
        ("perspective", 50.0),
        ("position.z", 0.0),
        ("anchor.z", 0.0),
        ("scale.z", 100.0),
        ("rotation.x", 0.0),
        ("rotation.y", 0.0),
        ("orientation.x", 0.0),
        ("orientation.y", 0.0),
        ("orientation.z", 0.0),
        ("anchor.x", 0.0),
        ("anchor.y", 0.0),
        ("position.x", 0.0),
        ("position.y", 0.0),
        ("scale.x", 100.0),
        ("scale.y", 100.0),
        ("rotation", 0.0),
        ("opacity", 100.0),
        ("skew", 0.0),
    ];
    let layers = project
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap();
    for l in layers {
        if l.get("type").and_then(Value::as_str) == Some("audio") {
            continue;
        }
        let m = l.as_object_mut().unwrap();
        if m.get("p").is_none() {
            m.insert("p", Value::Object(Map::new()));
        }
        let p = m.get_mut("p").unwrap().as_object_mut().unwrap();
        for (k, v) in defaults {
            if p.get(k).is_none() {
                p.insert(
                    k,
                    parse(&format!(r#"{{"v": {v}, "kf": [], "expr": null}}"#)).unwrap(),
                );
            }
        }
    }
    DocumentStore::new_untitled(project)
}

fn layer(store: &DocumentStore, id: &str) -> Value {
    let root = store.root().unwrap();
    let loc = pm_mac_core::model::find_layer(root, id).unwrap_or_else(|| panic!("no layer {id}"));
    pm_mac_core::model::layer_at(root, &loc).unwrap().to_value()
}

fn ids(store: &DocumentStore) -> Vec<String> {
    store.layers().into_iter().map(|l| l.id).collect()
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

fn refused(store: &mut DocumentStore, c: impl Into<Command>, origin: Origin) -> CommandError {
    let before = store.revision();
    match store.execute(c.into(), origin) {
        Err(TransactionError::Command(e)) => {
            assert_eq!(store.revision(), before);
            e
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn patch(id: &str, p: LayerPatch) -> LayerCommand {
    LayerCommand::Set {
        layer_id: id.into(),
        patch: p,
    }
}

// ── set_layer ───────────────────────────────────────────────────────────────

#[test]
fn set_layer_writes_switches_and_moves_before_zero_keeping_local_keys() {
    let mut s = store();
    run(
        &mut s,
        patch(
            "top",
            LayerPatch {
                from: Some(-7.0),
                ..Default::default()
            },
        ),
    );
    let l = layer(&s, "top");
    assert_eq!((num(&l, &["from"]), num(&l, &["dur"])), (-7.0, 10.0));
    assert_eq!(
        num(&l, &["p", "opacity", "kf", "0", "t"]),
        2.0,
        "keys are layer-local"
    );
    s.undo().unwrap();
    assert_eq!(num(&layer(&s, "top"), &["from"]), 0.0);
    s.redo().unwrap();
    assert_eq!(num(&layer(&s, "top"), &["from"]), -7.0);

    run(
        &mut s,
        patch(
            "top",
            LayerPatch {
                solo: Some(true),
                shy: Some(true),
                blend: Some("screen".into()),
                duration: Some(0.0),
                ..Default::default()
            },
        ),
    );
    let l = layer(&s, "top");
    assert_eq!(get(&l, &["solo"]), &Value::Bool(true));
    assert_eq!(get(&l, &["blend"]).as_str(), Some("screen"));
    assert_eq!(num(&l, &["dur"]), 1.0 / 30.0, "at least one frame");
}

#[test]
fn set_layer_refusals_have_the_old_messages() {
    let mut s = store();
    let cases: Vec<(&str, LayerPatch, &str)> = vec![
        (
            "top",
            LayerPatch {
                blend: Some("glow".into()),
                ..Default::default()
            },
            "Unknown blend mode: glow",
        ),
        (
            "sound",
            LayerPatch {
                blend: Some("add".into()),
                ..Default::default()
            },
            "Audio layers do not support blend modes",
        ),
        (
            "top",
            LayerPatch {
                continuous_raster: Some(true),
                ..Default::default()
            },
            "Only pre-comp layers can be continuously rasterized",
        ),
        (
            "top",
            LayerPatch {
                parent: Some(ParentChange {
                    parent: Some("top".into()),
                    time: 0.0,
                }),
                ..Default::default()
            },
            "Invalid parent layer",
        ),
        (
            "top",
            LayerPatch {
                parent: Some(ParentChange {
                    parent: Some("sound".into()),
                    time: 0.0,
                }),
                ..Default::default()
            },
            "This layer cannot be a parent",
        ),
        (
            "locked",
            LayerPatch {
                name: Some("x".into()),
                ..Default::default()
            },
            "Layer “Locked” is locked",
        ),
    ];
    for (id, p, message) in cases {
        let e = refused(&mut s, patch(id, p), Origin::Interface);
        assert_eq!(e.to_string(), message);
    }
    // A cycle through the parent chain.
    run(
        &mut s,
        patch(
            "box",
            LayerPatch {
                parent: Some(ParentChange {
                    parent: Some("top".into()),
                    time: 0.0,
                }),
                ..Default::default()
            },
        ),
    );
    let e = refused(
        &mut s,
        patch(
            "top",
            LayerPatch {
                parent: Some(ParentChange {
                    parent: Some("box".into()),
                    time: 0.0,
                }),
                ..Default::default()
            },
        ),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Parenting would create a cycle");
    // Humans may unlock; agents may not.
    let unlock = patch(
        "locked",
        LayerPatch {
            locked: Some(false),
            ..Default::default()
        },
    );
    assert!(matches!(
        refused(&mut s, unlock.clone(), Origin::Agent),
        CommandError::Locked { .. }
    ));
    run(&mut s, unlock);
    assert_eq!(get(&layer(&s, "locked"), &["lock"]), &Value::Bool(false));
}

#[test]
fn reparenting_keeps_the_world_position() {
    let mut s = store();
    // top sits at x 10; box at x 50. Parented to top, box's local x is 40.
    run(
        &mut s,
        patch(
            "box",
            LayerPatch {
                parent: Some(ParentChange {
                    parent: Some("top".into()),
                    time: 0.0,
                }),
                ..Default::default()
            },
        ),
    );
    let b = layer(&s, "box");
    assert_eq!(get(&b, &["parent"]).as_str(), Some("top"));
    assert!((num(&b, &["p", "position.x", "v"]) - 40.0).abs() < 1e-9);
    assert_eq!(s.history().len(), 1, "one undo step");
}

#[test]
fn track_mattes_refuse_cycles_and_default_to_alpha() {
    let mut s = store();
    run(
        &mut s,
        patch(
            "box",
            LayerPatch {
                matte_source: Some(Some("top".into())),
                ..Default::default()
            },
        ),
    );
    let b = layer(&s, "box");
    assert_eq!(get(&b, &["matteSource"]).as_str(), Some("top"));
    assert_eq!(get(&b, &["matteMode", "v"]).as_str(), Some("alpha"));
    let e = refused(
        &mut s,
        patch(
            "top",
            LayerPatch {
                matte_source: Some(Some("box".into())),
                ..Default::default()
            },
        ),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Invalid matte source or matte cycle");
}

// ── add / delete / duplicate / paste / order ────────────────────────────────

fn add(id: &str, kind: &str) -> AddLayer {
    AddLayer {
        id: id.into(),
        layer_type: kind.into(),
        comp: None,
        name: None,
        from: None,
        duration: None,
        content: Map::new(),
        properties: Map::new(),
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time: 2.51,
    }
}

#[test]
fn add_layer_uses_mk_layer_defaults_and_the_playhead() {
    let mut s = store();
    let mut c = add("new", "shape");
    c.properties.insert("opacity", Value::Number(40.0));
    run(&mut s, LayerCommand::Add(Box::new(c)));
    assert_eq!(ids(&s)[0], "new", "on top by default");
    let l = layer(&s, "new");
    assert_eq!(get(&l, &["name"]).as_str(), Some("Shape"));
    assert_eq!(num(&l, &["from"]), 2.5, "the playhead snapped to a frame");
    assert_eq!(num(&l, &["dur"]), 7.5, "to the end of the composition");
    assert_eq!(num(&l, &["p", "position.x", "v"]), 100.0, "centered");
    assert_eq!(num(&l, &["p", "opacity", "v"]), 40.0);
    assert_eq!(num(&l, &["d", "w"]), 480.0);
    assert_eq!(
        num(&l, &["p", "perspective", "v"]),
        50.0,
        "3D channels exist"
    );
    // Before zero, explicit duration.
    let mut early = add("early", "solid");
    early.from = Some(-3.0);
    early.duration = Some(5.0);
    early.index = Some(99);
    run(&mut s, LayerCommand::Add(Box::new(early)));
    let l = layer(&s, "early");
    assert_eq!((num(&l, &["from"]), num(&l, &["dur"])), (-3.0, 5.0));
    assert_eq!(
        ids(&s).last().map(String::as_str),
        Some("early"),
        "index clamped"
    );
    assert_eq!(
        num(&l, &["p", "position.x", "v"]),
        0.0,
        "solids sit at the origin"
    );

    let e = refused(
        &mut s,
        LayerCommand::Add(Box::new(add("top", "shape"))),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Layer id already exists: top");
    let e = refused(
        &mut s,
        LayerCommand::Add(Box::new(add("x", "blob"))),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Unknown layer type: blob");
    let mut audio = add("a", "audio");
    audio.blend = Some("add".into());
    let e = refused(
        &mut s,
        LayerCommand::Add(Box::new(audio)),
        Origin::Interface,
    );
    assert_eq!(
        e.to_string(),
        "Audio layers do not support parenting, blend modes, or motion blur"
    );
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(ids(&s), ids(&store()));
}

#[test]
fn delete_skips_locked_layers_and_unparents_children() {
    let mut s = store();
    run(
        &mut s,
        patch(
            "box",
            LayerPatch {
                parent: Some(ParentChange {
                    parent: Some("top".into()),
                    time: 0.0,
                }),
                ..Default::default()
            },
        ),
    );
    run(
        &mut s,
        LayerCommand::Delete {
            layer_ids: vec!["top".into(), "locked".into()],
        },
    );
    assert!(!ids(&s).contains(&"top".to_owned()));
    assert!(
        ids(&s).contains(&"locked".to_owned()),
        "locked layers are skipped"
    );
    assert!(get(&layer(&s, "box"), &["parent"]).is_null());
    let e = refused(
        &mut s,
        LayerCommand::Delete {
            layer_ids: vec!["locked".into()],
        },
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "All targeted layers are locked: “Locked”");
    s.undo().unwrap();
    assert_eq!(ids(&s), ids(&store()));
}

#[test]
fn duplicate_inserts_numbered_copies_above_with_fresh_keys() {
    let mut s = store();
    run(
        &mut s,
        LayerCommand::Duplicate {
            layer_ids: vec!["clip".into()],
            new_ids: vec!["clip2".into()],
        },
    );
    let order = ids(&s);
    let at = order.iter().position(|i| i == "clip2").unwrap();
    assert_eq!(order[at + 1], "clip");
    let c = layer(&s, "clip2");
    assert_eq!(get(&c, &["name"]).as_str(), Some("Clip 2"));
    assert_eq!(num(&c, &["from"]), 2.0, "in place");
    let k = get(&c, &["p", "opacity", "kf", "0", "i"]).as_str().unwrap();
    assert_ne!(k, "kc1", "fresh key ids");
    let e = refused(
        &mut s,
        LayerCommand::Duplicate {
            layer_ids: vec!["locked".into()],
            new_ids: vec!["l2".into()],
        },
        Origin::Interface,
    );
    assert!(matches!(e, CommandError::Locked { .. }));
}

#[test]
fn paste_moves_the_block_to_the_playhead_and_remaps_links() {
    let mut s = store();
    let mut a = layer(&s, "top");
    let mut b = layer(&s, "box");
    if let Some(m) = b.as_object_mut() {
        m.insert("parent", Value::String("top".into()));
        m.insert("matteSource", Value::String("top".into()));
    }
    if let Some(m) = a.as_object_mut() {
        m.insert("from", Value::Number(3.0));
    }
    run(
        &mut s,
        LayerCommand::Paste {
            comp: None,
            layers: vec![a, b],
            new_ids: vec!["pa".into(), "pb".into()],
            index: Some(1),
            time: Some(5.0),
        },
    );
    let order = ids(&s);
    assert_eq!(&order[1..3], &["pa".to_owned(), "pb".to_owned()]);
    assert_eq!(
        num(&layer(&s, "pa"), &["from"]),
        7.0,
        "the block starts at the box (1 s → 5 s)"
    );
    assert_eq!(num(&layer(&s, "pb"), &["from"]), 5.0);
    assert_eq!(get(&layer(&s, "pb"), &["parent"]).as_str(), Some("pa"));
    assert_eq!(get(&layer(&s, "pb"), &["matteSource"]).as_str(), Some("pa"));
}

#[test]
fn reorder_and_restack() {
    let mut s = store();
    run(
        &mut s,
        LayerCommand::Reorder {
            layer_id: "top".into(),
            index: 2,
            time: 0.0,
        },
    );
    assert_eq!(&ids(&s)[..3], &["box", "clip", "top"]);
    // Bring forward steps past one unselected unlocked layer.
    run(
        &mut s,
        LayerCommand::Restack {
            layer_ids: vec!["sound".into()],
            to: Restack::Forward,
        },
    );
    assert_eq!(&ids(&s)[..4], &["box", "clip", "sound", "top"]);
    run(
        &mut s,
        LayerCommand::Restack {
            layer_ids: vec!["sound".into()],
            to: Restack::Front,
        },
    );
    assert_eq!(ids(&s)[0], "sound");
    assert_eq!(s.history().undo_label(), Some("Bring to front"));
    // A locked layer is a barrier: "bottom" cannot pass "locked".
    let before = s.revision();
    run(
        &mut s,
        LayerCommand::Restack {
            layer_ids: vec!["bottom".into()],
            to: Restack::Front,
        },
    );
    assert_eq!(s.revision(), before, "nothing moved");
    assert_eq!(ids(&s).last().map(String::as_str), Some("bottom"));
}

// ── timing ──────────────────────────────────────────────────────────────────

#[test]
fn trim_in_keeps_media_frames_and_clamps_to_the_source() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::TrimIn {
            layer_ids: vec!["clip".into()],
            time: 3.0,
        },
    );
    let c = layer(&s, "clip");
    assert_eq!((num(&c, &["from"]), num(&c, &["dur"])), (3.0, 3.0));
    assert_eq!(num(&c, &["d", "trim"]), 2.0, "1 s of trim + 1 s trimmed");
    // Earlier than the source start (from 3, trim 2 → 1 s) is clamped.
    run(
        &mut s,
        TimingCommand::TrimIn {
            layer_ids: vec!["clip".into()],
            time: 0.0,
        },
    );
    let c = layer(&s, "clip");
    assert_eq!((num(&c, &["from"]), num(&c, &["d", "trim"])), (1.0, 0.0));
    run(
        &mut s,
        TimingCommand::TrimOut {
            layer_ids: vec!["clip".into(), "locked".into()],
            time: 0.0,
        },
    );
    assert_eq!(
        num(&layer(&s, "clip"), &["dur"]),
        1.0 / 30.0,
        "one frame remains"
    );
    assert_eq!(
        num(&layer(&s, "locked"), &["dur"]),
        10.0,
        "locked layers are skipped"
    );
}

#[test]
fn moves_brackets_slip_and_stagger() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::Move {
            layer_ids: vec!["box".into()],
            delta: 0.5,
        },
    );
    assert_eq!(num(&layer(&s, "box"), &["from"]), 1.5);
    run(
        &mut s,
        TimingCommand::MoveOutTo {
            layer_ids: vec!["box".into()],
            time: 10.0,
        },
    );
    assert_eq!(num(&layer(&s, "box"), &["from"]), 6.0);
    run(
        &mut s,
        TimingCommand::MoveInTo {
            layer_ids: vec!["box".into()],
            time: 0.0,
        },
    );
    assert_eq!(num(&layer(&s, "box"), &["from"]), 0.0);
    run(
        &mut s,
        TimingCommand::Slip {
            layer_id: "clip".into(),
            source_in: 3.0,
        },
    );
    assert_eq!(num(&layer(&s, "clip"), &["d", "trim", "v"]), 3.0);
    let e = refused(
        &mut s,
        TimingCommand::Slip {
            layer_id: "clip".into(),
            source_in: 5.0,
        },
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Slip exceeds available source handles.");
    run(
        &mut s,
        TimingCommand::Stagger {
            layer_ids: vec!["bottom".into(), "top".into(), "box".into()],
            step: 0.25,
        },
    );
    assert_eq!(num(&layer(&s, "bottom"), &["from"]), 0.0);
    assert_eq!(num(&layer(&s, "top"), &["from"]), 0.25);
    assert_eq!(num(&layer(&s, "box"), &["from"]), 0.5);
    assert!(matches!(
        refused(
            &mut s,
            TimingCommand::Move {
                layer_ids: vec!["locked".into()],
                delta: 1.0
            },
            Origin::Interface
        ),
        CommandError::Locked { .. }
    ));
}

#[test]
fn split_makes_a_continuous_tail_above_the_head() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::Split {
            layer_ids: vec!["clip".into(), "sound".into(), "bottom".into()],
            time: 3.0,
            new_ids: vec!["clipB".into(), "soundB".into(), "bottomB".into()],
        },
    );
    let order = ids(&s);
    let at = order.iter().position(|i| i == "clipB").unwrap();
    assert_eq!(order[at + 1], "clip", "the tail sits above its head");
    let head = layer(&s, "clip");
    let tail = layer(&s, "clipB");
    assert_eq!((num(&head, &["from"]), num(&head, &["dur"])), (2.0, 1.0));
    assert_eq!((num(&tail, &["from"]), num(&tail, &["dur"])), (3.0, 3.0));
    assert_eq!(num(&tail, &["d", "trim"]), 2.0, "the source continues");
    // Keys re-based: head key at local 1 (comp 3) is local 0 on the tail.
    let tk: Vec<f64> = get(&tail, &["p", "opacity", "kf"])
        .as_array()
        .unwrap()
        .iter()
        .map(|k| num(k, &["t"]))
        .collect();
    assert_eq!(tk, vec![0.0, 2.0]);
    let st = layer(&s, "soundB");
    assert_eq!(num(&st, &["d", "fadeIn"]), 0.0);
    assert_eq!(num(&layer(&s, "sound"), &["d", "fadeOut"]), 0.0);
    assert_eq!(s.history().len(), 1);
    s.undo().unwrap();
    assert_eq!(ids(&s), ids(&store()));
}

// ── groups ──────────────────────────────────────────────────────────────────

#[test]
fn group_and_ungroup_keep_poses_and_undo_in_one_step() {
    let mut s = store();
    run(
        &mut s,
        GroupCommand::Group {
            layer_ids: vec!["box".into(), "top".into()],
            group_id: "g".into(),
            name: Some("  ".into()),
            time: 0.0,
        },
    );
    let order = ids(&s);
    assert_eq!(
        &order[..3],
        &["g", "top", "box"],
        "the group sits above its first member"
    );
    let g = layer(&s, "g");
    assert_eq!(get(&g, &["name"]).as_str(), Some("Group"));
    assert_eq!(get(&layer(&s, "box"), &["group"]).as_str(), Some("g"));
    // The pivot moved to the members' bounds center, members unchanged.
    let gx = num(&g, &["p", "position.x", "v"]);
    assert_eq!(gx, num(&g, &["p", "anchor.x", "v"]));
    assert_eq!(num(&layer(&s, "box"), &["p", "position.x", "v"]), 50.0);
    run(
        &mut s,
        GroupCommand::Ungroup {
            group_ids: vec!["g".into()],
            time: 0.0,
        },
    );
    assert!(!ids(&s).contains(&"g".to_owned()));
    assert!(get(&layer(&s, "box"), &["group"]).is_null());
    assert_eq!(num(&layer(&s, "box"), &["p", "position.x", "v"]), 50.0);
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(ids(&s), ids(&store()));

    let e = refused(
        &mut s,
        GroupCommand::Group {
            layer_ids: vec!["locked".into()],
            group_id: "g2".into(),
            name: None,
            time: 0.0,
        },
        Origin::Interface,
    );
    assert!(matches!(e, CommandError::Locked { .. }));
    let e = refused(
        &mut s,
        GroupCommand::Ungroup {
            group_ids: vec!["top".into()],
            time: 0.0,
        },
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Select a group to ungroup");
}

#[test]
fn move_to_group_and_group_strips() {
    let mut s = store();
    run(
        &mut s,
        GroupCommand::Group {
            layer_ids: vec!["top".into()],
            group_id: "g".into(),
            name: Some("Rig".into()),
            time: 0.0,
        },
    );
    run(
        &mut s,
        GroupCommand::MoveToGroup {
            layer_ids: vec!["box".into()],
            group: Some("g".into()),
            time: 0.0,
        },
    );
    assert_eq!(get(&layer(&s, "box"), &["group"]).as_str(), Some("g"));
    assert_eq!(
        &ids(&s)[..3],
        &["g", "top", "box"],
        "members follow their group"
    );
    // Moving the group strip moves its members.
    run(
        &mut s,
        TimingCommand::Move {
            layer_ids: vec!["g".into()],
            delta: 1.0,
        },
    );
    assert_eq!(num(&layer(&s, "box"), &["from"]), 2.0);
    assert_eq!(num(&layer(&s, "top"), &["from"]), 1.0);
    let e = refused(
        &mut s,
        GroupCommand::MoveToGroup {
            layer_ids: vec!["g".into()],
            group: Some("g".into()),
            time: 0.0,
        },
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Grouping would create a cycle");
}

#[test]
fn key_drags_restart_from_the_pointer_down_state() {
    let mut s = store();
    let g = s.begin_gesture("Move keyframe", Origin::Interface).unwrap();
    // Passing over kc2 (local 3) removes it; moving on brings it back.
    for delta in [2.0, 1.0] {
        s.update_gesture_from_start(
            g,
            &[KeyframeCommand::Move {
                key_ids: vec!["kc1".into()],
                delta,
            }
            .into()],
        )
        .unwrap();
    }
    s.commit_gesture(g).unwrap();
    let keys = get(&layer(&s, "clip"), &["p", "opacity", "kf"])
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(keys.len(), 2, "the passed key came back");
    assert_eq!(num(&keys[0], &["t"]), 2.0);
    assert_eq!(s.history().len(), 1);
}

#[test]
fn align_moves_the_common_box_to_the_frame_edge_once() {
    use pm_mac_core::TransformCommand;
    use pm_mac_core::commands::transform::AlignMode;
    let mut s = store();
    let align = || TransformCommand::Align {
        layer_ids: vec!["box".into(), "top".into()],
        mode: AlignMode::Right,
        time: 0.0,
    };
    let (bx, tx) = (
        num(&layer(&s, "box"), &["p", "position.x", "v"]),
        num(&layer(&s, "top"), &["p", "position.x", "v"]),
    );
    run(&mut s, align());
    let (bx2, tx2) = (
        num(&layer(&s, "box"), &["p", "position.x", "v"]),
        num(&layer(&s, "top"), &["p", "position.x", "v"]),
    );
    assert_ne!(bx, bx2, "the selection moved");
    assert!(
        ((bx2 - bx) - (tx2 - tx)).abs() < 1e-9,
        "both by the same delta"
    );
    let before = s.revision();
    run(&mut s, align());
    assert_eq!(s.revision(), before, "already aligned");
}
