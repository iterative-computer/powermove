use super::*;
use crate::surface_backing::mixer::*;
use std::collections::{BTreeMap, BTreeSet};
struct Source;
impl MixerSource for Source {
    fn evaluate(&self, _: &Value, raw: &Value, _: f64, _: &str) -> Result<Value> {
        Ok(if raw.get("v").is_some() {
            raw["v"].clone()
        } else {
            raw.clone()
        })
    }
    fn key_at(&self, _: &Value, raw: &Value, at: f64) -> Result<bool> {
        Ok(raw["kf"]
            .as_array()
            .is_some_and(|keys| keys.iter().any(|k| k["t"].as_f64() == Some(at))))
    }
    fn group_ancestors(&self, layer: &Value) -> Result<Vec<Value>> {
        Ok(layer["groups"].as_array().cloned().unwrap_or_default())
    }
}
fn strip() -> Strip {
    strips(&json!({"layers":[{"id":"a","type":"audio","name":"Speech","d":{"asset":"media","gain":1},"on":true}]}),&BTreeMap::new(),&Source,0.).unwrap().remove(0)
}
#[test]
fn mixer_nested_audible_layers_have_no_arbitrary_depth_limit() {
    let mut comps = BTreeMap::new();
    for n in 0..20 {
        comps.insert(
            format!("c{n}"),
            json!({"layers":[{"type":"precomp","d":{"comp":format!("c{}",n+1)}}]}),
        );
    }
    comps.insert(
        "c20".into(),
        json!({"layers":[{"type":"audio","d":{"asset":"sound"}}]}),
    );
    assert!(carries_audio(
        &json!({"type":"precomp","d":{"comp":"c0"}}),
        &comps
    ));
}
#[test]
fn silent_cycles_and_missing_media_do_not_create_strips() {
    let comps = BTreeMap::from([(
        "c".into(),
        json!({"layers":[{"type":"precomp","d":{"comp":"c"}}]}),
    )]);
    for l in [
        json!({"type":"precomp","d":{"comp":"c"}}),
        json!({"type":"audio","d":{"asset":""}}),
        json!({"type":"video","d":{"asset":"v","embeddedAudio":false}}),
    ] {
        assert!(!carries_audio(&l, &comps));
    }
}
#[test]
fn cycle_with_sound_in_another_branch_is_audible() {
    let comps = BTreeMap::from([(
        "c".into(),
        json!({"layers":[{"type":"precomp","d":{"comp":"c"}},{"type":"audio","d":{"asset":"a"}}]}),
    )]);
    assert!(carries_audio(
        &json!({"type":"precomp","d":{"comp":"c"}}),
        &comps
    ));
}
#[test]
fn strip_order_paths_animation_mute_and_group_locks_match_source() {
    let p = json!({"layers":[{"id":"v","type":"video","d":{"asset":"m","embeddedAudio":true,"audioGain":{"v":2,"kf":[{"t":3}]},"timeRemap":true},"on":true,"groups":[{"on":false,"lock":true}]},{"id":"a","type":"audio","d":{"asset":"a","gain":0.5},"on":{"v":false,"kf":[]}}]});
    let rows = strips(&p, &BTreeMap::new(), &Source, 3.).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["v", "a"]
    );
    assert_eq!(rows[0].gain_key(), "audioGain");
    assert!(rows[0].animated && rows[0].key_at_time && rows[0].locked);
    assert_eq!(rows[0].silence, Some(Silence::Off));
    assert!(rows[1].muted && rows[1].mute_animated);
}
#[test]
fn locked_gain_is_refused_but_static_mute_overrides_lock() {
    let mut s = strip();
    s.locked = true;
    assert!(level_command(Some(&s), 1., 0.).is_err());
    assert_eq!(
        mute_command(&s, 2.).unwrap(),
        json!({"type":"set_layer","target":"a","patch":{"visible":false},"overrideLock":true})
    );
    s.mute_animated = true;
    assert_eq!(mute_command(&s, 2.).unwrap()["time"], 2.);
}
#[test]
fn animated_gain_gesture_captures_original_time_and_target() {
    let mut s = strip();
    s.animated = true;
    let g = LevelGesture::new(session(1), 3, 1.25, Some(s)).unwrap();
    for gain in [0., 0.5, 2., 4.] {
        let c = g.command(&session(1), gain).unwrap();
        assert_eq!(c["time"], 1.25);
        assert_eq!(c["target"], "a");
        assert_eq!(c["mode"], "auto");
    }
    assert!(g.command(&session(2), 1.).is_err());
}
#[test]
fn static_and_master_gain_edits_use_different_commands() {
    assert_eq!(
        level_command(Some(&strip()), 2., 0.).unwrap()["type"],
        "set_content"
    );
    assert_eq!(
        level_command(None, 2., 0.).unwrap(),
        json!({"type":"set_composition","patch":{"audioGain":2.}})
    );
    assert!(level_command(None, 5., 0.).is_err());
    assert!(level_command(None, f64::NAN, 0.).is_err());
}
#[test]
fn gain_and_db_conversion_preserve_silence_and_ceiling() {
    assert_eq!(clamp_gain(f64::INFINITY), 4.);
    assert_eq!(clamp_gain(f64::NAN), 0.);
    assert_eq!(db_to_gain(-100.), 0.);
    assert_eq!(gain_to_db(0.), f64::NEG_INFINITY);
    for gain in [0.25, 0.5, 1., 2., 4.] {
        assert!((db_to_gain(gain_to_db(gain)) - gain).abs() < 1e-12);
    }
}
#[test]
fn monitor_solo_is_exclusive_toggle_and_never_document_data() {
    let mut l = Listening {
        panel_shown: true,
        window_visible: true,
        playing: true,
        audio_running: true,
        ..Default::default()
    };
    l.toggle("a", false);
    l.toggle("b", false);
    assert_eq!(l.solo.len(), 2);
    l.toggle("b", true);
    assert_eq!(l.solo, BTreeSet::from(["b".into()]));
    l.toggle("b", true);
    assert!(l.solo.is_empty());
    l.toggle("a", false);
    l.window_visible = false;
    l.reconcile(&BTreeSet::from(["a".into()]));
    assert_eq!(l.solo.len(), 1);
    assert!(!l.sample_meters());
    l.panel_shown = false;
    l.reconcile(&BTreeSet::from(["a".into()]));
    assert!(l.solo.is_empty());
}
#[test]
fn solo_reconciles_removed_layers_and_meter_sampling_requires_all_gates() {
    let mut l = Listening {
        panel_shown: true,
        window_visible: true,
        playing: true,
        audio_running: true,
        ..Default::default()
    };
    l.toggle("gone", false);
    l.reconcile(&BTreeSet::new());
    assert!(l.solo.is_empty());
    assert!(l.sample_meters());
    l.audio_running = false;
    assert!(!l.sample_meters());
}
#[test]
fn meters_attack_fall_integrate_and_latch_clips() {
    let mut m = Meter::default();
    m.step(1., 0.5, 0.1).unwrap();
    assert_eq!(m.peak_db, 0.);
    assert!(m.clipped);
    assert!(m.power > 0. && m.power < 0.25);
    let hold = m.hold_db;
    m.step(0., 0., 0.25).unwrap();
    assert_eq!(m.peak_db, -5.);
    assert_eq!(m.hold_db, hold);
    assert!(m.clipped);
    m.clear_clip();
    assert!(!m.clipped);
    m.step(1., 1., 0.1).unwrap();
    m.playback_started();
    assert!(!m.clipped);
}
#[test]
fn invalid_meter_samples_leave_last_real_readout_unchanged() {
    let mut m = Meter::default();
    m.step(0.5, 0.2, 0.1).unwrap();
    let before = m.clone();
    assert!(m.step(f64::NAN, 0., 0.1).is_err());
    assert_eq!(m, before);
    assert!(m.step(1., 1e300, 0.1).is_err());
    assert_eq!(m, before);
}
