//! Layer, timing and group commands against the old app's semantics
//! (`editing.ts` `setLayer`/`addLayer`/`deleteLayers`/`reorderLayer`,
//! `model.ts`, `shortcuts.ts` paste/duplicate/order/split, the timeline's
//! trims, `layer-groups.ts`).

use pm_kernel::commands::keyframes::KeyframeCommand;
use pm_kernel::commands::layers::{AddLayer, BlockLayer, LayerPatch, ParentChange, Restack};
use pm_kernel::{
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
    store_with(|_| {})
}

fn store_with(change: impl FnOnce(&mut Value)) -> DocumentStore {
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
    change(&mut project);
    DocumentStore::new_untitled(project)
}

fn layer(store: &DocumentStore, id: &str) -> Value {
    let root = store.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, id).unwrap_or_else(|| panic!("no layer {id}"));
    pm_kernel::model::layer_at(root, &loc).unwrap().to_value()
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
    run(
        &mut s,
        LayerCommand::Delete {
            layer_ids: vec!["locked".into()],
        },
    );
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
fn an_inserted_block_keeps_names_group_local_poses_and_locks_in_one_step() {
    let mut s = store();
    let before = s.revision();
    let mut group = add("fg", "group");
    group.name = Some("Card".into());
    let mut child = add("fr", "shape");
    child.name = Some("Top".into());
    child.properties.insert("position.x", Value::Number(12.0));
    let block = |group: AddLayer, child: AddLayer, of: &str| LayerCommand::InsertBlock {
        label: "Paste Figma layers".into(),
        layers: vec![
            BlockLayer {
                add: group,
                group: None,
                locked: false,
            },
            BlockLayer {
                add: child,
                group: Some(of.into()),
                locked: true,
            },
        ],
    };
    let e = refused(
        &mut s,
        block(group.clone(), child.clone(), "top"),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Unknown group layer: top");
    run(&mut s, block(group, child, "fg"));
    assert_eq!(s.revision(), before + 1);
    assert_eq!(&ids(&s)[..3], &["fg", "fr", "top"]);
    let fr = layer(&s, "fr");
    // Not renumbered like Paste, and not moved to keep a world pose.
    assert_eq!(get(&fr, &["name"]).as_str(), Some("Top"));
    assert_eq!(num(&fr, &["p", "position.x", "v"]), 12.0);
    assert_eq!(get(&fr, &["group"]).as_str(), Some("fg"));
    assert_eq!(get(&fr, &["lock"]).as_bool(), Some(true));
    assert_eq!(get(&layer(&s, "fg"), &["name"]).as_str(), Some("Card"));
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

fn projected_anchor(s: &DocumentStore, id: &str) -> (f64, f64) {
    let loc = pm_kernel::model::find_layer(s.root().unwrap(), id).unwrap();
    let snapshot = s.snapshot();
    let mut edit =
        pm_edit::Edit::new(&snapshot.document, "C1", "C1", None, Default::default()).unwrap();
    edit.read(|scope| {
        let l = &scope.layers()[loc.index];
        let x = scope.value3d(l, "anchor.x", 0.0, 0.0)?;
        let y = scope.value3d(l, "anchor.y", 0.0, 0.0)?;
        let m = scope.plane_matrix(l, 0.0, false)?;
        Ok(pm_eval::space3d::project_point(&m, x, y))
    })
    .unwrap()
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
}

#[test]
fn anchor_align_single_off_centre_and_one_named_undo_step() {
    use pm_kernel::{TransformCommand, commands::transform::AlignMode};
    let mut s = store_with(|p| {
        let l = &mut p
            .as_object_mut()
            .unwrap()
            .get_mut("layers")
            .unwrap()
            .as_array_mut()
            .unwrap()[1];
        l.as_object_mut()
            .unwrap()
            .get_mut("p")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("anchor.x", parse(r#"{"v":137,"kf":[]}"#).unwrap());
    });
    let original = layer(&s, "box");
    run(
        &mut s,
        TransformCommand::Align {
            layer_ids: vec!["box".into()],
            mode: AlignMode::HorizontalCenter,
            time: 0.0,
        },
    );
    close(projected_anchor(&s, "box").0, 100.0);
    assert_eq!(s.history().len(), 1);
    assert_eq!(s.history().undo_label(), Some("Align Horizontal Centres"));
    s.undo().unwrap();
    assert_eq!(layer(&s, "box"), original);
}

#[test]
fn anchor_align_selection_left_skips_locked_and_is_idempotent() {
    use pm_kernel::{TransformCommand, commands::transform::AlignMode};
    let mut s = store();
    let original = layer(&s, "locked");
    let command = || TransformCommand::Align {
        layer_ids: vec!["box".into(), "locked".into(), "top".into()],
        mode: AlignMode::Left,
        time: 0.0,
    };
    run(&mut s, command());
    close(projected_anchor(&s, "box").0, 10.0);
    close(projected_anchor(&s, "top").0, 10.0);
    assert_eq!(layer(&s, "locked"), original);
    assert_eq!(s.history().len(), 1);
    let before = s.revision();
    run(&mut s, command());
    assert_eq!(s.revision(), before);
}

#[test]
fn anchor_align_parent_and_child_3d_projection() {
    use pm_kernel::{TransformCommand, commands::transform::AlignMode};
    for three_d in [false, true] {
        let mut s = store_with(|p| {
            let layers = p
                .as_object_mut()
                .unwrap()
                .get_mut("layers")
                .unwrap()
                .as_array_mut()
                .unwrap();
            let parent = layers[0].as_object_mut().unwrap();
            parent.insert("threeD", Value::Bool(three_d));
            let props = parent.get_mut("p").unwrap().as_object_mut().unwrap();
            for (key, v) in [
                ("rotation", 27.0),
                ("rotation.y", 32.0),
                ("position.z", 130.0),
                ("scale.x", 170.0),
            ] {
                props.insert(key, parse(&format!(r#"{{"v":{v},"kf":[]}}"#)).unwrap());
            }
            let child = layers[1].as_object_mut().unwrap();
            child.insert("parent", Value::from("top"));
            child.insert("threeD", Value::Bool(three_d));
        });
        let a = projected_anchor(&s, "top");
        let b = projected_anchor(&s, "box");
        let target = (a.0 + b.0) / 2.0;
        // Child first in selection deliberately exercises ancestor ordering.
        run(
            &mut s,
            TransformCommand::Align {
                layer_ids: vec!["box".into(), "top".into()],
                mode: AlignMode::HorizontalCenter,
                time: 0.0,
            },
        );
        close(projected_anchor(&s, "top").0, target);
        close(projected_anchor(&s, "box").0, target);
        close(projected_anchor(&s, "top").1, a.1);
        close(projected_anchor(&s, "box").1, b.1);
        assert_eq!(s.history().len(), 1);
    }
}

#[test]
fn anchor_distribute_spaces_anchors_and_undoes_once() {
    use pm_kernel::{TransformCommand, commands::transform::Spread};
    let mut s = store();
    let original = layer(&s, "top");
    run(
        &mut s,
        TransformCommand::Distribute {
            layer_ids: vec!["box".into(), "bottom".into(), "top".into()],
            spread: Spread::HorizontalSpace,
            time: 0.0,
        },
    );
    close(projected_anchor(&s, "bottom").0, 0.0);
    close(projected_anchor(&s, "top").0, 25.0);
    close(projected_anchor(&s, "box").0, 50.0);
    assert_eq!(s.history().len(), 1);
    assert_eq!(s.history().undo_label(), Some("Distribute Horizontally"));
    s.undo().unwrap();
    assert_eq!(layer(&s, "top"), original);
}

#[test]
fn anchor_align_all_edges_centres_and_group_anchor() {
    use pm_kernel::{TransformCommand, commands::transform::AlignMode};
    for (mode, expected, horizontal) in [
        (AlignMode::Left, 0., true),
        (AlignMode::Right, 200., true),
        (AlignMode::HorizontalCenter, 100., true),
        (AlignMode::Top, 0., false),
        (AlignMode::Bottom, 100., false),
        (AlignMode::VerticalCenter, 50., false),
    ] {
        let mut s = store_with(|p| {
            let layers = p
                .as_object_mut()
                .unwrap()
                .get_mut("layers")
                .unwrap()
                .as_array_mut()
                .unwrap();
            layers[0]
                .as_object_mut()
                .unwrap()
                .insert("type", Value::from("group"));
            layers[1]
                .as_object_mut()
                .unwrap()
                .insert("group", Value::from("top"));
        });
        run(
            &mut s,
            TransformCommand::Align {
                layer_ids: vec!["top".into()],
                mode,
                time: 0.0,
            },
        );
        let anchor = projected_anchor(&s, "top");
        close(if horizontal { anchor.0 } else { anchor.1 }, expected);
        // The member keeps its local Position when the group moves.
        close(num(&layer(&s, "box"), &["p", "position.x", "v"]), 50.);
    }
}

#[test]
fn anchor_distribute_parented_3d_on_both_axes() {
    use pm_kernel::{TransformCommand, commands::transform::Spread};
    for horizontal in [true, false] {
        let mut s = store_with(|p| {
            let layers = p
                .as_object_mut()
                .unwrap()
                .get_mut("layers")
                .unwrap()
                .as_array_mut()
                .unwrap();
            for i in [0, 1, 5] {
                let l = layers[i].as_object_mut().unwrap();
                l.insert("threeD", Value::Bool(true));
                let props = l.get_mut("p").unwrap().as_object_mut().unwrap();
                props.insert(
                    "position.z",
                    parse(&format!(r#"{{"v":{},"kf":[]}}"#, i * 90)).unwrap(),
                );
            }
            layers[1]
                .as_object_mut()
                .unwrap()
                .insert("parent", Value::from("top"));
        });
        let before: Vec<_> = ["top", "box", "bottom"]
            .iter()
            .map(|id| projected_anchor(&s, id))
            .collect();
        let coord = |p: &(f64, f64)| if horizontal { p.0 } else { p.1 };
        let lo = before.iter().map(coord).fold(f64::INFINITY, f64::min);
        let hi = before.iter().map(coord).fold(f64::NEG_INFINITY, f64::max);
        run(
            &mut s,
            TransformCommand::Distribute {
                layer_ids: vec!["box".into(), "top".into(), "bottom".into()],
                spread: if horizontal {
                    Spread::HorizontalSpace
                } else {
                    Spread::VerticalSpace
                },
                time: 0.0,
            },
        );
        let mut after: Vec<_> = ["top", "box", "bottom"]
            .iter()
            .map(|id| coord(&projected_anchor(&s, id)))
            .collect();
        after.sort_by(f64::total_cmp);
        for (k, value) in after.iter().enumerate() {
            close(*value, lo + (hi - lo) * k as f64 / 2.);
        }
        assert_eq!(s.history().len(), 1);
    }
}

fn delete_groups_store() -> DocumentStore {
    DocumentStore::new_untitled(
        parse(
            r#"{"id":"P","w":200,"h":100,"fps":30,"dur":10,"assets":{},"layers":[
      {"id":"g","type":"group","name":"Group","p":{}},
      {"id":"nested","type":"group","group":"g","p":{}},
      {"id":"child","type":"shape","group":"nested","p":{}},
      {"id":"other","type":"group","p":{}},
      {"id":"sibling","type":"shape","group":"other","p":{}},
      {"id":"locked","type":"shape","name":"Locked","lock":true,"p":{}},
      {"id":"pre","type":"precomp","d":{"comp":"C2"},"p":{}}
    ],"comps":{"C2":{"id":"C2","w":200,"h":100,"fps":30,"dur":10,"layers":[
      {"id":"inside","type":"group","p":{}},{"id":"inner","type":"shape","group":"inside","p":{}}
    ]}}}"#,
        )
        .unwrap(),
    )
}

#[test]
fn delete_groups_overlaps_different_parents_and_precomp_restore_in_one_undo() {
    for targets in [
        vec!["g", "nested", "child", "g"],
        vec!["nested", "sibling"],
        vec!["g", "inside", "inner"],
        vec!["pre", "inside", "g"],
        vec!["stale-after-undo", "g", "child"],
    ] {
        let mut s = delete_groups_store();
        let before = s.root().unwrap().to_value();
        run(
            &mut s,
            LayerCommand::Delete {
                layer_ids: targets.iter().map(|id| (*id).into()).collect(),
            },
        );
        for id in &targets {
            assert!(
                pm_kernel::model::find_layer(s.root().unwrap(), id).is_none(),
                "{id} survived {targets:?}"
            );
        }
        s.undo().unwrap();
        assert_eq!(s.root().unwrap().to_value(), before, "{targets:?}");
        s.redo().unwrap();
        s.undo().unwrap();
        assert_eq!(s.root().unwrap().to_value(), before);
    }
}

#[test]
fn delete_groups_skips_locked_instead_of_rejecting_the_selection() {
    let mut s = delete_groups_store();
    let before = s.root().unwrap().to_value();
    run(
        &mut s,
        LayerCommand::Delete {
            layer_ids: vec!["g".into(), "locked".into()],
        },
    );
    assert!(!ids(&s).contains(&"g".into()));
    assert!(ids(&s).contains(&"locked".into()));
    s.undo().unwrap();
    assert_eq!(s.root().unwrap().to_value(), before);
}

#[test]
fn delete_groups_transformed_group_with_locked_child_is_skipped_with_world_pose_unchanged() {
    let mut project = parse(PROJECT).unwrap();
    let layers = project
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap();
    // An actual translated, rotated, nonuniformly scaled group.
    layers[0]
        .as_object_mut()
        .unwrap()
        .insert("type", Value::String("group".into()));
    for (path, value) in [
        ("position.x", 230.),
        ("position.y", 80.),
        ("rotation", 35.),
        ("scale.x", 170.),
        ("scale.y", 65.),
    ] {
        layers[0]
            .as_object_mut()
            .unwrap()
            .get_mut("p")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("v", Value::Number(value));
    }
    layers[1]
        .as_object_mut()
        .unwrap()
        .insert("group", Value::String("top".into()));
    layers[1]
        .as_object_mut()
        .unwrap()
        .insert("lock", Value::Bool(true));
    let mut s = DocumentStore::new_untitled(project);
    let before = s.root().unwrap().to_value();
    let group = layer(&s, "top");
    let child = layer(&s, "box");
    let pose = s.queries().world_matrix(None, "box", 0.).unwrap();
    run(
        &mut s,
        LayerCommand::Delete {
            layer_ids: vec!["top".into(), "box".into(), "clip".into()],
        },
    );
    assert_eq!(layer(&s, "top"), group);
    assert_eq!(layer(&s, "box"), child);
    assert_eq!(s.queries().world_matrix(None, "box", 0.).unwrap(), pose);
    assert!(
        !ids(&s).contains(&"clip".into()),
        "unrelated editable layers still delete"
    );
    assert_eq!(s.history().len(), 1);
    s.undo().unwrap();
    assert_eq!(s.root().unwrap().to_value(), before);
}

// Timeline time is `from + local`; source time is `local * speed + trim`.
// Cutting an edge must preserve both mappings throughout the remaining span.
fn timing_fixture(kind: &str, remap: bool) -> DocumentStore {
    let mut p = parse(PROJECT).unwrap();
    let mut l = layer(&store(), "clip");
    let m = l.as_object_mut().unwrap();
    m.insert("type", Value::from(kind));
    m.insert("threeD", Value::Bool(true));
    m.insert("parent", Value::from("top"));
    m.insert("fx", parse(r#"[{"id":"f","type":"blur","on":true,"p":{"amount":{"v":1,"kf":[{"t":1,"v":2,"i":"fk"}]}}}]"#).unwrap());
    m.insert(
        "masks",
        parse(r#"[{"id":"m","p":{"opacity":{"v":100,"kf":[{"t":1,"v":50,"i":"mk"}]}}}]"#).unwrap(),
    );
    let d = m.get_mut("d").unwrap().as_object_mut().unwrap();
    d.insert("speed", Value::Number(2.0));
    if remap {
        d.insert("timeRemap", Value::Bool(true));
        d.insert(
            "sourceTime",
            parse(r#"{"v":0,"kf":[{"t":0,"v":1,"i":"r0"},{"t":4,"v":9,"i":"r1"}]}"#).unwrap(),
        );
    }
    p.as_object_mut()
        .unwrap()
        .insert("layers", Value::Array(vec![l]));
    if kind == "group" {
        let mut child = layer(&store(), "box");
        let m = child.as_object_mut().unwrap();
        m.insert("group", Value::from("clip"));
        m.insert("from", Value::Number(2.0));
        p.as_object_mut()
            .unwrap()
            .get_mut("layers")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(child);
    }
    DocumentStore::new_untitled(p)
}

fn timing_sample(s: &DocumentStore, id: &str, t: f64) -> (f64, f64) {
    let doc = pm_model::Document::from_root(Some(s.root().unwrap().clone()), s.revision());
    let comp = pm_eval::Comp::resolve(&doc, "C1").unwrap();
    let mut ev = pm_eval::Evaluator::new();
    let mut scope = pm_eval::Scope::open(&mut ev, &comp);
    let l = comp
        .layers()
        .iter()
        .find(|l| {
            l.get("id")
                .is_some_and(|v| v.to_value().as_str() == Some(id))
        })
        .unwrap();
    let source = scope.source_time(l, t, None).unwrap();
    let opacity = scope
        .channel_p(l, l.get("p").unwrap().get("opacity"), t, "opacity")
        .unwrap()
        .to_number();
    (source, opacity)
}

#[test]
fn split_every_kind_preserves_span_comp_keys_attributes_and_one_undo() {
    for kind in [
        "precomp", "video", "audio", "image", "shape", "text", "group", "solid",
    ] {
        for remap in [false, true] {
            if remap && !matches!(kind, "precomp" | "video") {
                continue;
            }
            let mut s = timing_fixture(kind, remap);
            let before = s.root().unwrap().to_value();
            let samples = [3. - 1. / 30., 3. + 1. / 30.].map(|t| timing_sample(&s, "clip", t));
            run(
                &mut s,
                TimingCommand::Split {
                    layer_ids: vec!["clip".into()],
                    time: 3.,
                    new_ids: vec!["tail".into()],
                },
            );
            let a = layer(&s, "clip");
            let b = layer(&s, "tail");
            assert_eq!((num(&a, &["from"]), num(&a, &["dur"])), (2., 1.), "{kind}");
            assert_eq!((num(&b, &["from"]), num(&b, &["dur"])), (3., 3.), "{kind}");
            let order = ids(&s);
            assert!(
                order.iter().position(|i| i == "tail").unwrap()
                    < order.iter().position(|i| i == "clip").unwrap()
            );
            for field in ["threeD", "parent", "fx", "masks"] {
                // Clone renews key IDs; values and comp times are checked separately.
                if field == "fx" || field == "masks" {
                    continue;
                }
                assert_eq!(a.get(field), b.get(field), "{kind}: {field}");
            }
            for path in [["p", "opacity"], ["fx", "0"], ["masks", "0"]] {
                let (ap, bp) = if path[0] == "p" {
                    (get(&a, &path), get(&b, &path))
                } else {
                    (
                        get(
                            &a,
                            &[
                                path[0],
                                "0",
                                "p",
                                if path[0] == "fx" { "amount" } else { "opacity" },
                            ],
                        ),
                        get(
                            &b,
                            &[
                                path[0],
                                "0",
                                "p",
                                if path[0] == "fx" { "amount" } else { "opacity" },
                            ],
                        ),
                    )
                };
                assert_eq!(
                    num(ap, &["kf", "0", "t"]) + 2.,
                    num(bp, &["kf", "0", "t"]) + 3.
                );
            }
            if matches!(kind, "precomp" | "video" | "audio") {
                for (i, (id, t)) in [("clip", 3. - 1. / 30.), ("tail", 3. + 1. / 30.)]
                    .into_iter()
                    .enumerate()
                {
                    let actual = timing_sample(&s, id, t);
                    assert!(
                        (actual.0 - samples[i].0).abs() < 1e-8,
                        "{kind} remap={remap}: {actual:?} != {:?}",
                        samples[i]
                    );
                    assert!((actual.1 - samples[i].1).abs() < 1e-8);
                }
            }
            assert_eq!(s.history().len(), 1);
            s.undo().unwrap();
            assert_eq!(s.root().unwrap().to_value(), before);
        }
    }
}

#[test]
fn trim_edges_keep_source_and_animation_while_move_shifts_both() {
    for kind in ["precomp", "video", "image", "shape", "text", "audio"] {
        for remap in [false, true] {
            if remap && !matches!(kind, "precomp" | "video") {
                continue;
            }
            let mut s = timing_fixture(kind, remap);
            let before = s.root().unwrap().to_value();
            let expected = timing_sample(&s, "clip", 4.);
            for cmd in [
                TimingCommand::TrimIn {
                    layer_ids: vec!["clip".into()],
                    time: 3.,
                },
                TimingCommand::TrimOut {
                    layer_ids: vec!["clip".into()],
                    time: 5.,
                },
            ] {
                run(&mut s, cmd);
                let actual = timing_sample(&s, "clip", 4.);
                if matches!(kind, "precomp" | "video" | "audio") {
                    assert!((actual.0 - expected.0).abs() < 1e-8, "{kind}: source");
                }
                assert!((actual.1 - expected.1).abs() < 1e-8, "{kind}: animation");
                s.undo().unwrap();
                assert_eq!(s.root().unwrap().to_value(), before);
            }
            run(
                &mut s,
                TimingCommand::Move {
                    layer_ids: vec!["clip".into()],
                    delta: 1.,
                },
            );
            assert_eq!(timing_sample(&s, "clip", 5.), expected, "{kind}: move");
            s.undo().unwrap();
            assert_eq!(s.root().unwrap().to_value(), before);
        }
    }
}

// Ignore regenerated layer/key identities while comparing all authored values
// and links (links are normalized to each layer's index in its composition).
fn comparable_comp(mut comp: Value) -> Value {
    let ids: std::collections::HashMap<String, String> = comp
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, l)| {
            (
                l.get("id").unwrap().as_str().unwrap().to_owned(),
                format!("layer-{i}"),
            )
        })
        .collect();
    fn visit(v: &mut Value, ids: &std::collections::HashMap<String, String>) {
        match v {
            Value::Object(m) => {
                for (k, v) in m.iter_mut() {
                    if k.as_str() == Some("i") {
                        *v = Value::from("key");
                    } else {
                        visit(v, ids);
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, ids);
                }
            }
            Value::String(s) => {
                if let Some(id) = ids.get(&s.to_string_lossy()) {
                    *v = Value::from(id.as_str());
                }
            }
            _ => {}
        }
    }
    let m = comp.as_object_mut().unwrap();
    m.remove("id");
    m.remove("name");
    visit(&mut comp, &ids);
    comp
}

#[test]
fn split_and_trim_render_identical_frames_around_cut_headlessly() {
    use pm_protocol::{
        color::Precision,
        common::{Size, Ticks},
    };
    use pm_render::{MapResolver, RenderRequest, Renderer};
    let temp = tempfile::tempdir().unwrap();
    let video = temp.path().join("frames.mp4");
    let image = temp.path().join("still.png");
    let ffmpeg = std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap_or_else(|| {
        std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
            .join("Library/Caches/powermove-engine/ffmpeg/out")
            .into_os_string()
    }))
    .join("bin/ffmpeg");
    for (path, args) in [
        (
            &video,
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=32x32:rate=30:duration=8",
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
            ],
        ),
        (
            &image,
            vec![
                "-f",
                "lavfi",
                "-i",
                "color=c=red:size=32x32",
                "-frames:v",
                "1",
                "-threads",
                "1",
            ],
        ),
    ] {
        assert!(
            std::process::Command::new("taskpolicy")
                .arg("-b")
                .arg(&ffmpeg)
                .args(["-v", "error", "-y"])
                .args(args)
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }
    let mut renderer = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(32, 32), 1., Precision::Float16);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("a1".into(), video);
    resolver.paths.insert("still".into(), image);
    for kind in ["precomp", "video", "image"] {
        let mut p = parse(PROJECT).unwrap();
        let mut l = layer(&store(), "clip");
        let m = l.as_object_mut().unwrap();
        m.insert("type", Value::from(kind));
        let d = match kind {
            "precomp" => r#"{"comp":"nested","w":32,"h":32,"trim":1}"#,
            "image" => r#"{"asset":"still","w":32,"h":32,"fit":"stretch"}"#,
            _ => r#"{"asset":"a1","w":32,"h":32,"fit":"stretch","trim":1,"speed":1}"#,
        };
        m.insert("d", parse(d).unwrap());
        let transform = m.get_mut("p").unwrap().as_object_mut().unwrap();
        transform.insert("position.x", parse(r#"{"v":16,"kf":[]}"#).unwrap());
        transform.insert("position.y", parse(r#"{"v":16,"kf":[]}"#).unwrap());
        let mut child = layer(&store(), "box");
        child
            .as_object_mut()
            .unwrap()
            .insert("from", Value::Number(0.));
        let cp = child
            .as_object_mut()
            .unwrap()
            .get_mut("p")
            .unwrap()
            .as_object_mut()
            .unwrap();
        cp.insert("position.x", parse(r#"{"v":16,"kf":[]}"#).unwrap());
        cp.insert("position.y", parse(r#"{"v":16,"kf":[]}"#).unwrap());
        cp.insert(
            "opacity",
            parse(r#"{"v":0,"kf":[{"t":0,"v":0,"i":"n0"},{"t":6,"v":100,"i":"n1"}]}"#).unwrap(),
        );
        let root = p.as_object_mut().unwrap();
        root.insert("w", Value::Number(32.));
        root.insert("h", Value::Number(32.));
        root.insert("layers", Value::Array(vec![l]));
        root.insert("assets", parse(r#"{"a1":{"id":"a1","kind":"video","w":32,"h":32,"dur":8,"fps":30,"ext":"mp4","timing":2},"still":{"id":"still","kind":"image","w":32,"h":32,"ext":"png"}}"#).unwrap());
        let mut nested = parse(r##"{"nested":{"id":"nested","name":"Nested","w":32,"h":32,"fps":30,"dur":10,"bg":"#000000","comps":{},"layers":[]}}"##).unwrap();
        nested
            .as_object_mut()
            .unwrap()
            .get_mut("nested")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("layers", Value::Array(vec![child]));
        // A nested precomp reference stays shared, matching Duplicate Composition.
        let mut grandchild = layer(&store(), "clip");
        let gm = grandchild.as_object_mut().unwrap();
        gm.insert("id", Value::from("grandchild"));
        gm.insert("type", Value::from("precomp"));
        gm.insert("on", Value::Bool(false));
        gm.insert("parent", Value::from("box"));
        gm.insert("d", parse(r#"{"comp":"shared","w":32,"h":32}"#).unwrap());
        nested
            .as_object_mut()
            .unwrap()
            .get_mut("nested")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .get_mut("layers")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(grandchild);
        nested.as_object_mut().unwrap().insert(
            "shared",
            parse(r#"{"id":"shared","name":"Shared","w":32,"h":32,"fps":30,"dur":10,"layers":[]}"#)
                .unwrap(),
        );
        root.insert("comps", nested);
        let mut s = DocumentStore::new_untitled(p);
        let before_split = s.root().unwrap().to_value();
        let mut render = |s: &DocumentStore, t: f64| {
            let doc = pm_model::Document::from_root(Some(s.root().unwrap().clone()), s.revision());
            let frame = renderer
                .render(
                    &doc,
                    "C1",
                    Ticks::new((t * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap(),
                    &request,
                    &resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{kind}: {:?}", frame.failures);
            let pixels = renderer.read_rgba(&frame).unwrap();
            renderer.recycle(frame);
            pixels
        };
        let times = [3. - 1. / 30., 3. + 1. / 30.];
        let original = times.map(|t| render(&s, t));
        assert!(
            original[0]
                .chunks_exact(4)
                .any(|p| p[0] > 0.01 || p[1] > 0.01 || p[2] > 0.01),
            "{kind}: fixture must render"
        );
        run(
            &mut s,
            TimingCommand::Split {
                layer_ids: vec!["clip".into()],
                time: 3.,
                new_ids: vec!["tail".into()],
            },
        );
        for (i, t) in times.into_iter().enumerate() {
            assert_eq!(render(&s, t), original[i], "{kind}: split at {t}");
        }
        if kind == "precomp" {
            let tail = layer(&s, "tail");
            let copy_id = get(&tail, &["d", "comp"]).as_str().unwrap().to_owned();
            assert_ne!(copy_id, "nested");
            let root = s.root().unwrap().to_value();
            let original_comp = get(&root, &["comps", "nested"]);
            let copy = get(&root, &["comps", &copy_id]);
            assert_eq!(get(copy, &["name"]).as_str(), Some("Nested 2"));
            assert_eq!(
                comparable_comp(original_comp.clone()),
                comparable_comp(copy.clone())
            );
            assert_eq!(
                get(copy, &["layers", "1", "d", "comp"]).as_str(),
                Some("shared")
            );
            let copied_child = get(copy, &["layers", "0", "id"])
                .as_str()
                .unwrap()
                .to_owned();
            run(
                &mut s,
                Command::SetLayerVisible {
                    layer_id: copied_child,
                    visible: false,
                },
            );
            assert_eq!(
                render(&s, times[0]),
                original[0],
                "editing B must leave A unchanged"
            );
            assert_ne!(
                render(&s, times[1]),
                original[1],
                "editing B changes only B"
            );
            s.undo().unwrap(); // child edit
        }
        s.undo().unwrap(); // split and duplicate composition together
        assert_eq!(s.root().unwrap().to_value(), before_split);
        for cmd in [
            TimingCommand::TrimIn {
                layer_ids: vec!["clip".into()],
                time: 3.,
            },
            TimingCommand::TrimOut {
                layer_ids: vec!["clip".into()],
                time: 4.,
            },
        ] {
            run(&mut s, cmd);
            assert_eq!(render(&s, times[1]), original[1], "{kind}: trim");
            s.undo().unwrap();
        }
    }
}

#[test]
fn split_and_trim_integrate_animated_video_speed() {
    for kind in ["video"] {
        let mut fixture = parse(PROJECT).unwrap();
        fixture.as_object_mut().unwrap().insert(
            "layers",
            Value::Array(vec![layer(&timing_fixture(kind, false), "clip")]),
        );
        let l = fixture
            .as_object_mut()
            .unwrap()
            .get_mut("layers")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .first_mut()
            .unwrap();
        l.as_object_mut()
            .unwrap()
            .get_mut("d")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(
                "speed",
                parse(r#"{"v":1,"kf":[{"t":0,"v":1,"i":"s0"},{"t":4,"v":3,"i":"s1"}]}"#).unwrap(),
            );
        let mut s = DocumentStore::new_untitled(fixture);
        let expected = timing_sample(&s, "clip", 4.);
        for split in [false, true] {
            let cmd = if split {
                TimingCommand::Split {
                    layer_ids: vec!["clip".into()],
                    time: 3.,
                    new_ids: vec!["tail".into()],
                }
            } else {
                TimingCommand::TrimIn {
                    layer_ids: vec!["clip".into()],
                    time: 3.,
                }
            };
            run(&mut s, cmd);
            let actual = timing_sample(&s, if split { "tail" } else { "clip" }, 4.);
            assert!(
                (actual.0 - expected.0).abs() < 1e-8,
                "{kind}: {actual:?} != {expected:?}"
            );
            s.undo().unwrap();
        }
    }
}
