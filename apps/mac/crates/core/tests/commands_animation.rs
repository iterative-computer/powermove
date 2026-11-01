//! Property and keyframe commands: `set_property` (static and keyed at the
//! playhead), expressions, the stopwatch, and every keyframe edit, against
//! the old app's semantics (`editing.ts`, `anim.ts`, `temporal-ease.ts`,
//! the timeline's dialogs).

use pm_mac_core::commands::keyframes::{
    CopiedKey, EaseSide, EaseSpec, HandleValue, Interpolation, KeyframeSpec, ReplaceKeyframes,
    TemporalEase,
};
use pm_mac_core::commands::properties::{KeyMode, SetProperty};
use pm_mac_core::{
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
    let loc = pm_mac_core::model::find_layer(root, id).unwrap();
    pm_mac_core::model::layer_at(root, &loc).unwrap().to_value()
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
fn unported_commands_say_so_and_change_nothing() {
    let mut s = store();
    let e = refused(
        &mut s,
        PropertyCommand::Reset {
            layer_id: "s1".into(),
            channel: "opacity".into(),
            time: 0.0,
        }
        .into(),
        Origin::Interface,
    );
    assert_eq!(e, CommandError::NotImplemented("reset_property"));
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
            scope: pm_mac_core::commands::keyframes::SimplifyScope::Layers(vec!["s1".into()]),
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
            scope: pm_mac_core::commands::keyframes::SimplifyScope::All,
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
    assert_eq!(s.history().undo_label(), Some("Edit curve"));
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
