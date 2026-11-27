//! Property and keyframe commands: `set_property` (static and keyed at the
//! playhead), expressions, the stopwatch, and every keyframe edit, against
//! the old app's semantics (`editing.ts`, `anim.ts`, `temporal-ease.ts`,
//! the timeline's dialogs).

use pm_kernel::commands::keyframes::{
    CopiedKey, EaseSide, EaseSpec, HandleValue, Interpolation, KeyframeSpec, ReplaceKeyframes,
    TemporalEase,
};
use pm_kernel::commands::properties::{KeyMode, SetProperty};
use pm_kernel::{
    Command, CommandError, DocumentStore, KeyframeCommand, Origin, PropertyCommand,
    TransactionError,
};
use pm_model::json::{Value, parse};

const PROJECT: &str = r##"{
  "id": "P", "name": "Test", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 4, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "s1", "type": "shape", "name": "Box", "from": 1, "dur": 3, "on": true,
     "d": {"shape": "rect", "color": "#ff0000", "w": 10, "h": 10, "radius": 0},
     "fx": [{"id": "fx1", "type": "blur", "on": true, "p": {"amount": {"v": 4, "kf": [], "expr": null}}}],
     "p": {"opacity": {"v": 100, "kf": [], "expr": null},
           "position.x": {"v": 0, "kf": [], "expr": null}}},
    {"id": "anim", "type": "shape", "name": "Animated", "from": 0, "dur": 4,
     "p": {"opacity": {"v": 100, "kf": [
        {"t": 0, "v": 0, "i": "k1", "inInterp": "linear", "outInterp": "linear",
         "inEase": {"speed": 0, "influence": 33.333333333333336}, "outEase": {"speed": 0, "influence": 33.333333333333336},
         "autoBezier": false, "continuous": false},
        {"t": 1, "v": 100, "i": "k2", "inInterp": "linear", "outInterp": "linear",
         "inEase": {"speed": 0, "influence": 33.333333333333336}, "outEase": {"speed": 0, "influence": 33.333333333333336},
         "autoBezier": false, "continuous": false}], "expr": null}}},
    {"id": "legacy", "type": "shape", "name": "Legacy keys", "from": 0, "dur": 4,
     "p": {"rotation": {"v": 0, "kf": [
        {"t": 0, "v": 0, "i": "L1", "eo": [0.42, 0], "ei": [1, 1], "hold": false},
        {"t": 2, "v": 90, "i": "L2", "eo": [0, 0], "ei": [0.58, 1], "hold": false}], "expr": null}}},
    {"id": "locked", "type": "shape", "name": "Locked", "lock": true, "from": 0, "dur": 4,
     "p": {"opacity": {"v": 100, "kf": [{"t": 0, "v": 5, "i": "kl"}], "expr": null}}},
    {"id": "marked", "type": "shape", "name": "Marked", "from": 0, "dur": 4,
     "locked_intent": {"opacity": {"by": "human"}},
     "p": {"opacity": {"v": 100, "kf": [], "expr": null}}},
    {"id": "video", "type": "video", "name": "Clip", "from": 0, "dur": 2,
     "d": {"asset": "a1", "fit": "cover", "trim": 0, "speed": 1, "w": 10, "h": 10}, "p": {}},
    {"id": "colors", "type": "shape", "name": "Colors", "from": 0, "dur": 4,
     "d": {"color": {"v": "#000000", "kf": [{"t": 0, "v": "#000000", "i": "c1", "eo": [0,0], "ei": [1,1], "hold": false}], "expr": null}},
     "p": {}}
  ]
}"##;

fn store() -> DocumentStore {
    DocumentStore::new_untitled(parse(PROJECT).unwrap())
}

fn layer(store: &DocumentStore, id: &str) -> Value {
    let root = store.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, id).unwrap();
    pm_kernel::model::layer_at(root, &loc).unwrap().to_value()
}

fn at<'v>(v: &'v Value, path: &[&str]) -> &'v Value {
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
    at(v, path).as_f64().unwrap()
}

fn keys(store: &DocumentStore, id: &str, channel: &str) -> Vec<Value> {
    at(&layer(store, id), &["p", channel, "kf"])
        .as_array()
        .unwrap()
        .clone()
}

fn set(id: &str, channel: &str, value: impl Into<Value>, time: f64) -> Command {
    PropertyCommand::Set(SetProperty::new(id, channel, value, time)).into()
}

fn run(store: &mut DocumentStore, c: Command) {
    store.execute(c, Origin::Interface).unwrap();
}

fn refused(store: &mut DocumentStore, c: Command, origin: Origin) -> CommandError {
    let before = store.revision();
    match store.execute(c, origin) {
        Err(TransactionError::Command(e)) => {
            assert_eq!(
                store.revision(),
                before,
                "a refused command changes nothing"
            );
            e
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ── set_property ────────────────────────────────────────────────────────────

#[test]
fn a_static_channel_takes_the_value_and_undo_restores_it() {
    let mut s = store();
    run(&mut s, set("s1", "opacity", 40.0, 2.0));
    assert_eq!(num(&layer(&s, "s1"), &["p", "opacity", "v"]), 40.0);
    assert!(
        keys(&s, "s1", "opacity").is_empty(),
        "auto mode does not start animation"
    );
    assert_eq!(s.history().undo_label(), Some("Change Opacity"));
    s.undo().unwrap();
    assert_eq!(num(&layer(&s, "s1"), &["p", "opacity", "v"]), 100.0);
}

fn text_store(paragraph: bool, x: f64, y: f64) -> DocumentStore {
    DocumentStore::new_untitled(
        parse(&format!(
            r#"{{
      "id":"P","compId":"C","w":200,"h":100,"fps":30,"dur":4,
      "layers":[{{"id":"t","type":"text","from":0,"dur":4,
        "d":{{"text":"Title","size":80,"paragraph":{paragraph},"boxWidth":200}},
        "p":{{"position.x":{{"v":100,"kf":[]}},"position.y":{{"v":50,"kf":[]}},
              "anchor.x":{{"v":{x},"kf":[]}},"anchor.y":{{"v":{y},"kf":[]}}}}
      }}]
    }}"#
        ))
        .unwrap(),
    )
}

#[test]
fn text_size_resizes_about_the_anchor() {
    for (paragraph, x, y, expected_x) in [
        (false, 0., 40., 0.),
        (false, 30., 60., 60.),
        (true, 30., 60., 30.),
    ] {
        let mut s = text_store(paragraph, x, y);
        run(&mut s, set("t", "c.size", 160., 0.));
        let resized = layer(&s, "t");
        assert_eq!(num(&resized, &["p", "anchor.x", "v"]), expected_x);
        assert_eq!(num(&resized, &["p", "anchor.y", "v"]), y * 2.);
        assert_eq!(num(&resized, &["p", "position.x", "v"]), 100.);
        assert_eq!(num(&resized, &["p", "position.y", "v"]), 50.);
    }
}

#[test]
fn text_size_keys_only_change_the_anchor_at_the_edited_time() {
    let mut s = text_store(false, 0., 40.);
    let key = |channel, value, time| {
        PropertyCommand::Set(SetProperty {
            mode: KeyMode::Keyframe,
            ..SetProperty::new("t", channel, value, time)
        })
        .into()
    };

    // One size key is still a constant size across the layer.
    run(&mut s, key("c.size", 160., 0.));
    assert_eq!(num(&layer(&s, "t"), &["p", "anchor.y", "v"]), 80.);
    // Adding and editing another size key must leave the static anchor alone.
    run(&mut s, key("c.size", 80., 1.));
    run(&mut s, set("t", "c.size", 120., 1.));
    assert_eq!(num(&layer(&s, "t"), &["p", "anchor.y", "v"]), 80.);

    run(&mut s, key("anchor.y", 80., 0.));
    run(&mut s, key("anchor.y", 60., 1.));
    let before = keys(&s, "t", "anchor.y");
    run(&mut s, set("t", "c.size", 240., 1.));
    let after = keys(&s, "t", "anchor.y");
    assert_eq!(after[0], before[0]);
    assert_eq!(num(&after[1], &["v"]), 120.);
}

#[test]
fn static_text_size_keys_an_animated_anchor_at_the_playhead() {
    let mut s = text_store(false, 0., 40.);
    for (time, value) in [(0., 40.), (2., 80.)] {
        run(
            &mut s,
            PropertyCommand::Set(SetProperty {
                mode: KeyMode::Keyframe,
                ..SetProperty::new("t", "anchor.y", value, time)
            })
            .into(),
        );
    }
    run(&mut s, set("t", "c.size", 160., 1.));
    let after = keys(&s, "t", "anchor.y");
    assert_eq!(after.len(), 3);
    for (key, (t, v)) in after.iter().zip([(0., 40.), (1., 120.), (2., 80.)]) {
        assert_eq!(num(key, &["t"]), t);
        assert_eq!(num(key, &["v"]), v);
    }
}

#[test]
fn text_size_preserves_the_legacy_font_bounds_anchor_fraction() {
    let mut s = text_store(false, 30., 60.);
    let mut patch = pm_model::Map::new();
    patch.insert(
        "fontAnchorBounds",
        parse(r#"{"x0":-10,"x1":70,"y0":20,"y1":100}"#).unwrap(),
    );
    run(
        &mut s,
        pm_kernel::ContentCommand::Set {
            layer_id: "t".into(),
            patch,
            time: 0.,
        }
        .into(),
    );
    let original = layer(&s, "t");
    run(&mut s, set("t", "c.size", 160., 0.));
    let resized = layer(&s, "t");
    for (axis, start, end) in [("anchor.x", "x0", "x1"), ("anchor.y", "y0", "y1")] {
        let fraction = |layer: &Value| {
            let lo = num(layer, &["d", "fontAnchorBounds", start]);
            let hi = num(layer, &["d", "fontAnchorBounds", end]);
            (num(layer, &["p", axis, "v"]) - lo) / (hi - lo)
        };
        assert_eq!(fraction(&resized), fraction(&original));
        assert_eq!(
            num(&resized, &["p", axis, "v"]),
            num(&original, &["p", axis, "v"]) * 2.
        );
    }
    s.undo().unwrap();
    assert_eq!(layer(&s, "t"), original);
}

#[test]
fn an_animated_channel_is_keyed_at_the_playhead_in_layer_time() {
    let mut s = store();
    // Layer "anim" starts at 0; 0.51 s snaps to frame 15 (0.5 s).
    run(&mut s, set("anim", "opacity", 50.0, 0.51));
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 3);
    assert_eq!(num(&k[1], &["t"]), 0.5);
    assert_eq!(num(&k[1], &["v"]), 50.0);
    assert_eq!(at(&k[1], &["inInterp"]).as_str(), Some("linear"));
    assert!(
        k[1].get("eo").is_none(),
        "numeric keys are written in native form"
    );
    // Within half a frame of an existing key: that key is updated.
    run(&mut s, set("anim", "opacity", 60.0, 1.0 + 0.01));
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 3);
    assert_eq!(num(&k[2], &["v"]), 60.0);
    assert_eq!(
        at(&k[2], &["i"]).as_str(),
        Some("k2"),
        "the key keeps its id"
    );
}

#[test]
fn keyframe_mode_starts_animation_at_local_time() {
    let mut s = store();
    let mut c = SetProperty::new("s1", "opacity", 10.0, 2.0);
    c.mode = KeyMode::Keyframe;
    run(&mut s, PropertyCommand::Set(c).into());
    let k = keys(&s, "s1", "opacity");
    assert_eq!(k.len(), 1);
    assert_eq!(num(&k[0], &["t"]), 1.0, "layer s1 starts at 1 s");
    assert_eq!(
        num(&layer(&s, "s1"), &["p", "opacity", "v"]),
        100.0,
        "v is untouched"
    );
}

#[test]
fn a_named_ease_shapes_the_new_key_against_its_neighbours() {
    let mut s = store();
    let mut c = SetProperty::new("anim", "opacity", 50.0, 2.0);
    c.ease = Some(EaseSpec::Preset("easeInOut".into()));
    run(&mut s, PropertyCommand::Set(c).into());
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 3);
    assert_eq!(at(&k[2], &["inInterp"]).as_str(), Some("bezier"));
    assert_eq!(at(&k[2], &["outInterp"]).as_str(), Some("bezier"));
}

#[test]
fn numbers_are_coerced_like_number_and_non_finite_values_refused() {
    let mut s = store();
    run(&mut s, set("s1", "opacity", "25", 0.0));
    assert_eq!(num(&layer(&s, "s1"), &["p", "opacity", "v"]), 25.0);
    let e = refused(
        &mut s,
        set("s1", "opacity", f64::NAN, 0.0),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "opacity must be a finite number");
    let e = refused(&mut s, set("s1", "opacity", "abc", 0.0), Origin::Interface);
    assert!(matches!(e, CommandError::Invalid(_)));
}

#[test]
fn the_old_refusals_apply() {
    let mut s = store();
    assert!(matches!(
        refused(&mut s, set("nope", "opacity", 1.0, 0.0), Origin::Interface),
        CommandError::LayerNotFound(_)
    ));
    let e = refused(
        &mut s,
        set("locked", "opacity", 1.0, 0.0),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Layer “Locked” is locked");
    let e = refused(&mut s, set("s1", "nonsense", 1.0, 0.0), Origin::Interface);
    assert_eq!(e.to_string(), "Property “nonsense” was not found on “Box”");
    // Hand-edit markers: preserved by default for everyone, always for
    // extensions; humans and agents may explicitly overwrite.
    assert!(matches!(
        refused(
            &mut s,
            set("marked", "opacity", 1.0, 0.0),
            Origin::Interface
        ),
        CommandError::PreservedHandEdit { .. }
    ));
    let mut overwrite = SetProperty::new("marked", "opacity", 1.0, 0.0);
    overwrite.preserve_hand_edits = false;
    assert!(matches!(
        refused(
            &mut s,
            PropertyCommand::Set(overwrite.clone()).into(),
            Origin::Extension
        ),
        CommandError::PreservedHandEdit { .. }
    ));
    s.execute(PropertyCommand::Set(overwrite).into(), Origin::Agent)
        .unwrap();
    assert_eq!(num(&layer(&s, "marked"), &["p", "opacity", "v"]), 1.0);
}

#[test]
fn prefixed_paths_and_effect_channels_resolve() {
    let mut s = store();
    run(&mut s, set("s1", "transform.position.x", 12.0, 0.0));
    assert_eq!(num(&layer(&s, "s1"), &["p", "position.x", "v"]), 12.0);
    run(&mut s, set("s1", "fx1.amount", 9.0, 0.0));
    assert_eq!(num(&layer(&s, "s1"), &["fx", "0", "p", "amount", "v"]), 9.0);
    // `$enabled` and content channels become channels when first set.
    run(&mut s, set("s1", "fx1.$enabled", false, 0.0));
    assert_eq!(
        at(&layer(&s, "s1"), &["fx", "0", "on", "v"]),
        &Value::Bool(false)
    );
    run(&mut s, set("s1", "c.w", 33.0, 0.0));
    assert_eq!(num(&layer(&s, "s1"), &["d", "w", "v"]), 33.0);
}

#[test]
fn a_constant_video_speed_rescales_the_layer() {
    let mut s = store();
    run(&mut s, set("video", "c.speed", 2.0, 0.0));
    let v = layer(&s, "video");
    assert_eq!(num(&v, &["d", "speed", "v"]), 2.0);
    assert_eq!(num(&v, &["dur"]), 1.0, "2 s at 1× becomes 1 s at 2×");
}

#[test]
fn mark_intent_records_a_hand_edit() {
    let mut s = store();
    let mut c = SetProperty::new("s1", "opacity", 70.0, 1.5);
    c.mark_intent = true;
    run(&mut s, PropertyCommand::Set(c).into());
    let l = layer(&s, "s1");
    assert_eq!(
        at(&l, &["locked_intent", "opacity", "by"]).as_str(),
        Some("human")
    );
    assert_eq!(num(&l, &["locked_intent", "opacity", "t"]), 0.5);
}

#[test]
fn a_scrub_gesture_is_one_undo_step_and_creates_one_key() {
    let mut s = store();
    let g = s
        .begin_gesture("Change Opacity", Origin::Interface)
        .unwrap();
    for v in [10.0, 20.0, 30.0] {
        s.update_gesture(g, &[set("anim", "opacity", v, 2.0)])
            .unwrap();
    }
    s.commit_gesture(g).unwrap();
    let k = keys(&s, "anim", "opacity");
    assert_eq!(
        k.len(),
        3,
        "the key made by the first step is updated by the next"
    );
    assert_eq!(num(&k[2], &["v"]), 30.0);
    s.undo().unwrap();
    assert_eq!(keys(&s, "anim", "opacity").len(), 2);
}

// ── expressions and the stopwatch ───────────────────────────────────────────

#[test]
fn expressions_are_checked_and_removed_by_empty_strings() {
    let mut s = store();
    let set_expr = |e: Option<&str>| -> Command {
        PropertyCommand::SetExpression {
            layer_id: "s1".into(),
            channel: "opacity".into(),
            expression: e.map(str::to_owned),
        }
        .into()
    };
    run(&mut s, set_expr(Some("time * 10")));
    assert_eq!(
        at(&layer(&s, "s1"), &["p", "opacity", "expr"]).as_str(),
        Some("time * 10")
    );
    assert!(matches!(
        refused(&mut s, set_expr(Some("time *")), Origin::Interface),
        CommandError::Invalid(_)
    ));
    run(&mut s, set_expr(Some("")));
    assert!(at(&layer(&s, "s1"), &["p", "opacity", "expr"]).is_null());
}

#[test]
fn the_stopwatch_keys_a_static_value_and_bakes_an_animated_one() {
    let mut s = store();
    let toggle = |id: &str, t: f64| -> Command {
        PropertyCommand::ToggleStopwatch {
            layer_id: id.into(),
            channel: "opacity".into(),
            time: t,
        }
        .into()
    };
    run(&mut s, toggle("s1", 2.0));
    let k = keys(&s, "s1", "opacity");
    assert_eq!((num(&k[0], &["t"]), num(&k[0], &["v"])), (1.0, 100.0));
    // Animated: static at the value shown at the playhead (linear 0 → 100).
    run(&mut s, toggle("anim", 0.25));
    assert!(keys(&s, "anim", "opacity").is_empty());
    assert_eq!(num(&layer(&s, "anim"), &["p", "opacity", "v"]), 25.0);
}

// ── keyframes ───────────────────────────────────────────────────────────────

fn kc(c: KeyframeCommand) -> Command {
    c.into()
}

#[test]
fn the_diamond_adds_the_current_value_and_removes_the_key() {
    let mut s = store();
    let toggle = |t: f64| {
        kc(KeyframeCommand::Toggle {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            time: t,
            key_id: "new".into(),
            value: None,
        })
    };
    run(&mut s, toggle(0.5));
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 3);
    assert_eq!(at(&k[1], &["i"]).as_str(), Some("new"));
    assert_eq!(
        num(&k[1], &["v"]),
        50.0,
        "the evaluated value at the playhead"
    );
    run(&mut s, toggle(0.5));
    assert_eq!(keys(&s, "anim", "opacity").len(), 2);
}

#[test]
fn deleting_the_last_key_keeps_its_value() {
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::Delete {
            key_ids: vec!["k1".into(), "k2".into()],
        }),
    );
    assert!(keys(&s, "anim", "opacity").is_empty());
    assert_eq!(
        num(&layer(&s, "anim"), &["p", "opacity", "v"]),
        0.0,
        "kf[0].v"
    );
    s.undo().unwrap();
    assert_eq!(keys(&s, "anim", "opacity").len(), 2);
}

#[test]
fn deleting_all_keys_at_playhead_keeps_the_value_shown_at_the_playhead() {
    // Playhead ahead of the last keyframe keeps the last keyframe's value.
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::DeleteAt {
            key_ids: vec!["k1".into(), "k2".into()],
            time: 3.0,
        }),
    );
    assert!(keys(&s, "anim", "opacity").is_empty());
    assert_eq!(num(&layer(&s, "anim"), &["p", "opacity", "v"]), 100.0);

    // Playhead between keyframes keeps the interpolated value shown in the viewer.
    s.undo().unwrap();
    run(
        &mut s,
        kc(KeyframeCommand::DeleteAt {
            key_ids: vec!["k1".into(), "k2".into()],
            time: 0.75,
        }),
    );
    assert!(keys(&s, "anim", "opacity").is_empty());
    assert_eq!(num(&layer(&s, "anim"), &["p", "opacity", "v"]), 75.0);

    // Playhead before the first keyframe keeps the first keyframe's value.
    s.undo().unwrap();
    run(
        &mut s,
        kc(KeyframeCommand::DeleteAt {
            key_ids: vec!["k1".into(), "k2".into()],
            time: 0.0,
        }),
    );
    assert!(keys(&s, "anim", "opacity").is_empty());
    assert_eq!(num(&layer(&s, "anim"), &["p", "opacity", "v"]), 0.0);

    // Deleting only one of two keyframes keeps the remaining keyframe intact.
    s.undo().unwrap();
    run(
        &mut s,
        kc(KeyframeCommand::DeleteAt {
            key_ids: vec!["k1".into()],
            time: 3.0,
        }),
    );
    let remaining = keys(&s, "anim", "opacity");
    assert_eq!(remaining.len(), 1);
    assert_eq!(at(&remaining[0], &["i"]).as_str(), Some("k2"));
}

#[test]
fn clear_all_keeps_the_value_shown_at_the_playhead() {
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::ClearAll {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            time: 0.75,
        }),
    );
    assert!(keys(&s, "anim", "opacity").is_empty());
    assert_eq!(num(&layer(&s, "anim"), &["p", "opacity", "v"]), 75.0);
}

#[test]
fn moving_keys_replaces_a_key_it_lands_on() {
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::Move {
            key_ids: vec!["k1".into()],
            delta: 1.0,
        }),
    );
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 1);
    assert_eq!(at(&k[0], &["i"]).as_str(), Some("k1"));
    assert_eq!(num(&k[0], &["t"]), 1.0);
    // Never before zero.
    run(
        &mut s,
        kc(KeyframeCommand::Move {
            key_ids: vec!["k1".into()],
            delta: -5.0,
        }),
    );
    assert_eq!(num(&keys(&s, "anim", "opacity")[0], &["t"]), 0.0);
}

#[test]
fn the_value_dialog_snaps_times_and_refuses_collisions() {
    let mut s = store();
    let edit = |t: Option<f64>, v: Option<f64>| {
        kc(KeyframeCommand::Edit {
            key_id: "k2".into(),
            time: t,
            value: v.map(Value::Number),
        })
    };
    run(&mut s, edit(Some(2.01), Some(42.0)));
    let k = keys(&s, "anim", "opacity");
    assert_eq!((num(&k[1], &["t"]), num(&k[1], &["v"])), (2.0, 42.0));
    let e = refused(&mut s, edit(Some(0.01), None), Origin::Interface);
    assert_eq!(
        e.to_string(),
        "Another keyframe already sits on that frame."
    );
    let e = refused(&mut s, edit(Some(9.0), None), Origin::Interface);
    assert_eq!(e.to_string(), "That time falls outside the composition.");
    assert!(matches!(
        refused(
            &mut s,
            kc(KeyframeCommand::Edit {
                key_id: "gone".into(),
                time: None,
                value: Some(Value::Number(1.0))
            }),
            Origin::Interface
        ),
        CommandError::KeyframeNotFound(_)
    ));
}

#[test]
fn set_easing_turns_both_sides_into_bezier() {
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::SetEasing {
            key_ids: vec!["k1".into(), "k2".into()],
            curve: EaseSpec::Preset("easeInOut".into()),
        }),
    );
    let k = keys(&s, "anim", "opacity");
    for key in &k {
        assert_eq!(at(key, &["outInterp"]).as_str(), Some("bezier"));
        assert_eq!(at(key, &["inInterp"]).as_str(), Some("bezier"));
    }
    assert!(num(&k[0], &["outEase", "influence"]) > 33.0);
    let e = refused(
        &mut s,
        kc(KeyframeCommand::SetEasing {
            key_ids: vec!["gone".into()],
            curve: EaseSpec::Curve([0.3, 0.0, 0.7, 1.0]),
        }),
        Origin::Interface,
    );
    assert_eq!(
        e.to_string(),
        "The selected keyframes are no longer available"
    );
    let e = refused(
        &mut s,
        kc(KeyframeCommand::SetEasing {
            key_ids: vec!["kl".into()],
            curve: EaseSpec::linear(),
        }),
        Origin::Interface,
    );
    assert!(matches!(e, CommandError::Locked { .. }));
}

#[test]
fn legacy_handles_are_adopted_before_an_interpolation_edit() {
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::EasyEase {
            key_ids: vec!["L2".into()],
            side: EaseSide::Both,
        }),
    );
    let k = at(&layer(&s, "legacy"), &["p", "rotation", "kf"])
        .as_array()
        .unwrap()
        .clone();
    // L1 (not selected) was converted from its legacy handles too.
    assert!(k[0].get("eo").is_none());
    assert_eq!(at(&k[0], &["outInterp"]).as_str(), Some("bezier"));
    assert_eq!(at(&k[1], &["inInterp"]).as_str(), Some("bezier"));
    assert!((num(&k[1], &["inEase", "influence"]) - 100.0 / 3.0).abs() < 1e-9);
    assert_eq!(num(&k[1], &["inEase", "speed"]), 0.0);
}

#[test]
fn interpolation_hold_and_handles() {
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::SetHold {
            key_ids: vec!["k1".into()],
            hold: true,
        }),
    );
    assert_eq!(
        at(&keys(&s, "anim", "opacity")[0], &["outInterp"]).as_str(),
        Some("hold")
    );
    run(
        &mut s,
        kc(KeyframeCommand::SetInterpolation {
            key_ids: vec!["k1".into()],
            side: EaseSide::Out,
            interpolation: Interpolation::Linear,
        }),
    );
    assert_eq!(
        at(&keys(&s, "anim", "opacity")[0], &["outInterp"]).as_str(),
        Some("linear")
    );
    run(
        &mut s,
        kc(KeyframeCommand::SetInterpolation {
            key_ids: vec!["k2".into()],
            side: EaseSide::Both,
            interpolation: Interpolation::Continuous,
        }),
    );
    run(
        &mut s,
        kc(KeyframeCommand::SetHandle {
            key_id: "k2".into(),
            side: EaseSide::In,
            value: HandleValue::Ease(TemporalEase {
                speed: 50.0,
                influence: 60.0,
            }),
            split: false,
        }),
    );
    let k2 = &keys(&s, "anim", "opacity")[1];
    assert_eq!(num(k2, &["inEase", "speed"]), 50.0);
    assert_eq!(num(k2, &["inEase", "influence"]), 60.0);
    assert_eq!(
        num(k2, &["outEase", "speed"]),
        50.0,
        "a continuous key mirrors speed"
    );
    // A point handle: half the segment out, 75 units up from k1 (value 0).
    run(
        &mut s,
        kc(KeyframeCommand::SetHandle {
            key_id: "k1".into(),
            side: EaseSide::Out,
            value: HandleValue::Point { t: 0.5, v: 75.0 },
            split: true,
        }),
    );
    let k1 = &keys(&s, "anim", "opacity")[0];
    assert_eq!(num(k1, &["outEase", "influence"]), 50.0);
    assert_eq!(num(k1, &["outEase", "speed"]), 150.0);
}

#[test]
fn paste_places_keys_at_the_target_time_and_replaces_frames() {
    let mut s = store();
    let copied = keys(&s, "anim", "opacity");
    run(
        &mut s,
        kc(KeyframeCommand::Paste {
            layer_id: "s1".into(),
            channel: "opacity".into(),
            time: 2.0,
            keys: vec![
                CopiedKey {
                    at: 0.0,
                    key: copied[0].clone(),
                },
                CopiedKey {
                    at: 1.0,
                    key: copied[1].clone(),
                },
            ],
            new_ids: vec!["p1".into(), "p2".into()],
        }),
    );
    let k = keys(&s, "s1", "opacity");
    assert_eq!(k.len(), 2);
    // Composition 2 s and 3 s on a layer starting at 1 s.
    assert_eq!((num(&k[0], &["t"]), num(&k[1], &["t"])), (1.0, 2.0));
    assert_eq!(at(&k[0], &["i"]).as_str(), Some("p1"));
    // A color track refuses numeric keys.
    let e = refused(
        &mut s,
        kc(KeyframeCommand::Paste {
            layer_id: "colors".into(),
            channel: "c.color".into(),
            time: 0.0,
            keys: vec![CopiedKey {
                at: 0.0,
                key: copied[0].clone(),
            }],
            new_ids: vec!["p3".into()],
        }),
        Origin::Interface,
    );
    assert_eq!(
        e.to_string(),
        "No matching property to paste keyframes into"
    );
}

#[test]
fn replace_keyframes_rebuilds_the_track() {
    let mut s = store();
    let mut spec = KeyframeSpec::new(0.0, 10.0);
    spec.ease = Some(EaseSpec::Preset("easeOut".into()));
    run(
        &mut s,
        kc(KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            keyframes: vec![spec, KeyframeSpec::new(2.0, 90.0)],
            replace: true,
            expression: None,
            preserve_hand_edits: true,
            time: 0.0,
        }))),
    );
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 2);
    assert_eq!((num(&k[0], &["v"]), num(&k[1], &["t"])), (10.0, 2.0));
    assert_eq!(at(&k[0], &["outInterp"]).as_str(), Some("bezier"));
    // Replacing with nothing keeps the value shown at `time`.
    run(
        &mut s,
        kc(KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            keyframes: vec![],
            replace: true,
            expression: None,
            preserve_hand_edits: true,
            time: 2.0,
        }))),
    );
    assert!(keys(&s, "anim", "opacity").is_empty());
    assert_eq!(num(&layer(&s, "anim"), &["p", "opacity", "v"]), 90.0);
}

#[test]
fn native_replace_keeps_ids_and_refuses_duplicates() {
    let mut s = store();
    let mut a = KeyframeSpec::new(0.25, 1.0);
    a.id = Some("k1".into());
    let mut b = KeyframeSpec::new(0.5, 2.0);
    b.id = Some("k1".into());
    let e = refused(
        &mut s,
        kc(KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            keyframes: vec![a.clone(), b],
            replace: true,
            expression: None,
            preserve_hand_edits: true,
            time: 0.0,
        }))),
        Origin::Interface,
    );
    assert_eq!(e.to_string(), "Keyframe IDs must be unique within a track");
    run(
        &mut s,
        kc(KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            keyframes: vec![a],
            replace: false,
            expression: None,
            preserve_hand_edits: true,
            time: 0.0,
        }))),
    );
    let k = keys(&s, "anim", "opacity");
    assert_eq!(
        k.len(),
        2,
        "k1 moved to 0.25 s (exact subframe time); k2 kept"
    );
    assert_eq!(num(&k[0], &["t"]), 0.25);
    assert_eq!(at(&k[1], &["i"]).as_str(), Some("k2"));
}

#[test]
fn simplify_drops_only_constant_interior_keys() {
    let mut s = store();
    let mut keys_in = Vec::new();
    for (t, v) in [
        (0.0, 0.0),
        (1.0, 50.0),
        (1.5, 50.0),
        (2.0, 50.0),
        (3.0, 100.0),
    ] {
        keys_in.push(KeyframeSpec::new(t, v));
    }
    run(
        &mut s,
        kc(KeyframeCommand::Replace(Box::new(ReplaceKeyframes {
            layer_id: "s1".into(),
            channel: "opacity".into(),
            keyframes: keys_in,
            replace: true,
            expression: None,
            preserve_hand_edits: true,
            time: 0.0,
        }))),
    );
    run(
        &mut s,
        kc(KeyframeCommand::Simplify {
            scope: pm_kernel::commands::keyframes::SimplifyScope::Layers(vec!["s1".into()]),
        }),
    );
    let k = keys(&s, "s1", "opacity");
    let times: Vec<f64> = k.iter().map(|k| num(k, &["t"])).collect();
    assert_eq!(times, vec![0.0, 1.0, 2.0, 3.0], "the flat middle key goes");
    assert_eq!(s.history().undo_label(), Some("Simplify Keyframes"));
    // Any expression in the project keeps every key.
    let mut s = store();
    run(
        &mut s,
        PropertyCommand::SetExpression {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            expression: Some("value".into()),
        }
        .into(),
    );
    let before = s.revision();
    run(
        &mut s,
        kc(KeyframeCommand::Simplify {
            scope: pm_kernel::commands::keyframes::SimplifyScope::All,
        }),
    );
    assert_eq!(s.revision(), before);
}

#[test]
fn graph_moves_stop_a_frame_before_keys_they_do_not_own() {
    let mut s = store();
    // k1 (0 s) towards k2 (1 s): stops at 29/30 s, never replacing it.
    run(
        &mut s,
        kc(KeyframeCommand::GraphMove {
            key_ids: vec!["k1".into()],
            delta: 5.0,
            value_delta: 10.0,
        }),
    );
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 2);
    assert_eq!(num(&k[0], &["t"]), 29.0 / 30.0);
    assert_eq!(num(&k[0], &["v"]), 10.0);
    assert_eq!(s.history().undo_label(), Some("Edit Curve"));
}

#[test]
fn the_transform_box_scales_times_values_and_speeds() {
    let mut s = store();
    // Give k1 a speed to follow the scale.
    run(
        &mut s,
        kc(KeyframeCommand::SetHandle {
            key_id: "k1".into(),
            side: EaseSide::Out,
            value: HandleValue::Ease(TemporalEase {
                speed: 40.0,
                influence: 50.0,
            }),
            split: false,
        }),
    );
    run(
        &mut s,
        kc(KeyframeCommand::Transform {
            key_ids: vec!["k1".into(), "k2".into()],
            anchor_time: 0.0,
            time_scale: 2.0,
            anchor_value: 0.0,
            value_scale: 0.5,
        }),
    );
    let k = keys(&s, "anim", "opacity");
    assert_eq!((num(&k[1], &["t"]), num(&k[1], &["v"])), (2.0, 50.0));
    assert_eq!(num(&k[0], &["outEase", "speed"]), 10.0, "speed × 0.5 / 2");
    assert_eq!(
        num(&k[0], &["outEase", "influence"]),
        50.0,
        "influence is a ratio"
    );
    // Never mirrored through the anchor.
    run(
        &mut s,
        kc(KeyframeCommand::Transform {
            key_ids: vec!["k1".into(), "k2".into()],
            anchor_time: 0.0,
            time_scale: -1.0,
            anchor_value: 0.0,
            value_scale: 1.0,
        }),
    );
    let k = keys(&s, "anim", "opacity");
    assert_eq!(
        num(&k[1], &["t"]),
        1.0 / 30.0,
        "clamped to a frame from the anchor key"
    );
}

#[test]
fn selected_property_keys_take_absolute_value_and_undo_together() {
    let mut s = store();
    s.select_keys(["k1", "k2", "L1", "L2"]);
    let before = layer(&s, "anim");
    let other = layer(&s, "legacy");
    run(&mut s, set("anim", "opacity", 42.0, 0.0));
    assert!(
        keys(&s, "anim", "opacity")
            .iter()
            .all(|k| num(k, &["v"]) == 42.0)
    );
    assert_eq!(layer(&s, "legacy"), other);
    s.undo().unwrap();
    assert_eq!(layer(&s, "anim"), before);
}

#[test]
fn selected_keys_away_from_playhead_do_not_change() {
    let mut s = store();
    s.select_keys(["k1", "k2"]);
    run(&mut s, set("anim", "opacity", 42.0, 0.5));
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 3);
    assert_eq!(num(&k[0], &["v"]), 0.0);
    assert_eq!(num(&k[1], &["v"]), 42.0);
    assert_eq!(num(&k[2], &["v"]), 100.0);
}

#[test]
fn unselected_playhead_key_and_layer_scoped_selection_stay_local() {
    let mut s = store();
    s.select_layer_keys(vec![
        ("legacy".into(), "k1".into()),
        ("anim".into(), "k2".into()),
    ]);
    run(&mut s, set("anim", "opacity", 42.0, 0.0));
    let k = keys(&s, "anim", "opacity");
    assert_eq!(num(&k[0], &["v"]), 42.0);
    assert_eq!(num(&k[1], &["v"]), 100.0);
}

#[test]
/// The keyframe menu checks the ease the chosen keys have: every preset the
/// menu offers reads back by name after Set Easing writes it (v1's
/// `Ease.nameOf` of the handles), and a held key has none.
#[test]
fn every_menu_preset_reads_back_by_name() {
    use pm_kernel::commands::keyframes::ease_preset;
    let mut s = store();
    for name in [
        "linear", "easeIn", "easeOut", "easeInOut", "swish", "power", "expoOut", "backOut", "snap",
        "glide",
    ] {
        run(
            &mut s,
            kc(KeyframeCommand::SetEasing {
                key_ids: vec!["k1".into(), "k2".into()],
                curve: EaseSpec::Preset(name.into()),
            }),
        );
        let k = keys(&s, "anim", "opacity");
        assert_eq!(ease_preset(&k, 0), Some(name), "first key, {name}");
        assert_eq!(ease_preset(&k, 1), Some(name), "second key, {name}");
    }
    // v1's stored handles (a legacy track) are named as they are.
    let legacy = at(&layer(&s, "legacy"), &["p", "rotation", "kf"])
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(ease_preset(&legacy, 0), Some("easeIn"));
    let mut held = keys(&s, "anim", "opacity");
    held[0]
        .as_object_mut()
        .unwrap()
        .insert("outInterp", Value::String("hold".into()));
    assert_eq!(ease_preset(&held, 0), None);
}

#[test]
fn placing_a_keyframe_inside_a_curved_segment_preserves_value_and_speed_curve() {
    let eval_at = |s: &DocumentStore, t: f64| -> f64 {
        let kfs = keys(s, "anim", "opacity");
        let node = pm_model::Node::from_value(Value::Array(kfs), 0);
        let mut springs = pm_eval::easing::SpringTables::new();
        pm_eval::keyframes::evaluate(Some(&node), t, false, &mut springs)
            .unwrap()
            .as_number()
            .unwrap()
    };

    for preset in ["easeOut", "expoOut", "easeInOut", "swish"] {
        let mut s = store();
        run(
            &mut s,
            kc(KeyframeCommand::SetEasing {
                key_ids: vec!["k1".into(), "k2".into()],
                curve: EaseSpec::Preset(preset.into()),
            }),
        );

        let samples: Vec<(f64, f64)> = (0..=100)
            .map(|i| {
                let t = i as f64 / 100.0;
                (t, eval_at(&s, t))
            })
            .collect();

        // Insert a keyframe in the middle of the curved segment (e.g. 0.4s -> 12/30s).
        let insert_t = 0.4;
        let eps = 1e-4;
        let speed_left_before = (eval_at(&s, insert_t) - eval_at(&s, insert_t - eps)) / eps;
        let speed_right_before = (eval_at(&s, insert_t + eps) - eval_at(&s, insert_t)) / eps;
        run(
            &mut s,
            kc(KeyframeCommand::Add {
                layer_id: "anim".into(),
                channel: "opacity".into(),
                time: insert_t,
                key_id: "mid".into(),
                value: None,
            }),
        );

        let k = keys(&s, "anim", "opacity");
        assert_eq!(k.len(), 3, "preset {preset}: key inserted");
        assert_eq!(at(&k[1], &["i"]).as_str(), Some("mid"));
        assert_eq!(
            at(&k[1], &["continuous"]).as_bool(),
            Some(true),
            "preset {preset}: inserted key is continuous bezier"
        );

        // Every sampled point along the curve must match before and after.
        for &(t, before) in &samples {
            let after = eval_at(&s, t);
            assert!(
                (after - before).abs() < 1e-4,
                "preset {preset} at t={t}: value diverged before={before} after={after}"
            );
        }

        // Numerical speed right across the newly placed key must match the original curve!
        let speed_left_after = (eval_at(&s, insert_t) - eval_at(&s, insert_t - eps)) / eps;
        let speed_right_after = (eval_at(&s, insert_t + eps) - eval_at(&s, insert_t)) / eps;
        assert!(
            (speed_left_after - speed_left_before).abs() < 0.05
                && (speed_right_after - speed_right_before).abs() < 0.05,
            "preset {preset}: speed changed across inserted key before=({speed_left_before}, {speed_right_before}) after=({speed_left_after}, {speed_right_after})"
        );
    }
}

#[test]
fn placing_a_keyframe_inside_a_hold_segment_keeps_it_held() {
    let mut s = store();
    run(
        &mut s,
        kc(KeyframeCommand::SetHold {
            key_ids: vec!["k1".into()],
            hold: true,
        }),
    );
    run(
        &mut s,
        kc(KeyframeCommand::Add {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            time: 0.5,
            key_id: "mid".into(),
            value: None,
        }),
    );
    let k = keys(&s, "anim", "opacity");
    assert_eq!(k.len(), 3);
    assert_eq!(num(&k[1], &["v"]), 0.0);
    assert_eq!(at(&k[1], &["outInterp"]).as_str(), Some("hold"));
}

#[test]
fn set_easing_on_consecutive_keys_preserves_outer_unselected_segments() {
    let mut s = store();
    // Create 4 keyframes: k1 (t=0, v=0), k2 (t=1, v=100), k3 (t=2, v=200), k4 (t=3, v=300)
    run(
        &mut s,
        kc(KeyframeCommand::Add {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            time: 2.0,
            key_id: "k3".into(),
            value: Some(Value::Number(200.0)),
        }),
    );
    run(
        &mut s,
        kc(KeyframeCommand::Add {
            layer_id: "anim".into(),
            channel: "opacity".into(),
            time: 3.0,
            key_id: "k4".into(),
            value: Some(Value::Number(300.0)),
        }),
    );
    // Ease k1 -> k2 with easeIn, and k3 -> k4 with snap.
    run(
        &mut s,
        kc(KeyframeCommand::SetEasing {
            key_ids: vec!["k1".into(), "k2".into()],
            curve: EaseSpec::Preset("easeIn".into()),
        }),
    );
    run(
        &mut s,
        kc(KeyframeCommand::SetEasing {
            key_ids: vec!["k3".into(), "k4".into()],
            curve: EaseSpec::Preset("snap".into()),
        }),
    );
    let before = keys(&s, "anim", "opacity");
    let k2_in_before = before[1].get("inEase").unwrap().clone();
    let k2_in_interp_before = before[1].get("inInterp").unwrap().clone();
    let k3_out_before = before[2].get("outEase").unwrap().clone();
    let k3_out_interp_before = before[2].get("outInterp").unwrap().clone();

    // Now select k2 and k3 and adjust easing between them to easeOut.
    run(
        &mut s,
        kc(KeyframeCommand::SetEasing {
            key_ids: vec!["k2".into(), "k3".into()],
            curve: EaseSpec::Preset("easeOut".into()),
        }),
    );
    let after = keys(&s, "anim", "opacity");
    // Outer segments k1 -> k2 (k2.inEase) and k3 -> k4 (k3.outEase) MUST be untouched!
    assert_eq!(after[1].get("inEase").unwrap(), &k2_in_before);
    assert_eq!(after[1].get("inInterp").unwrap(), &k2_in_interp_before);
    assert_eq!(after[2].get("outEase").unwrap(), &k3_out_before);
    assert_eq!(after[2].get("outInterp").unwrap(), &k3_out_interp_before);
    // Interior segment k2 -> k3 received easeOut ([0, 0, 0.58, 1] -> in influence 42%).
    assert_eq!(at(&after[1], &["outInterp"]).as_str(), Some("bezier"));
    assert_eq!(at(&after[2], &["inInterp"]).as_str(), Some("bezier"));
    assert!((num(&after[2], &["inEase", "influence"]) - 42.0).abs() < 1e-6);

    // Selecting 3 keys [k2, k3, k4] eases both k2->k3 and k3->k4 while still
    // leaving k1->k2 (k2.inEase) completely untouched!
    run(
        &mut s,
        kc(KeyframeCommand::SetEasing {
            key_ids: vec!["k2".into(), "k3".into(), "k4".into()],
            curve: EaseSpec::Curve([0.8, 0.0, 0.2, 1.0]),
        }),
    );
    let multi = keys(&s, "anim", "opacity");
    assert_eq!(multi[1].get("inEase").unwrap(), &k2_in_before);
    assert!((num(&multi[1], &["outEase", "influence"]) - 80.0).abs() < 1e-6);
    assert!((num(&multi[2], &["inEase", "influence"]) - 80.0).abs() < 1e-6);
    assert!((num(&multi[2], &["outEase", "influence"]) - 80.0).abs() < 1e-6);
    assert!((num(&multi[3], &["inEase", "influence"]) - 80.0).abs() < 1e-6);
}
