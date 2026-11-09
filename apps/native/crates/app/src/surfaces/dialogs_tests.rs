#![allow(unused_imports)]
use super::kernel::*;
use super::*;
use pm_kernel::{
    Call, Caller, HistoryCall, KernelActor, KernelConfig, KernelHandle, OpenSource, Reply, Request,
};
use pm_model::{Value, json::parse};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

fn id() -> DialogId {
    DialogId("dialog".into())
}
fn owner() -> WindowId {
    WindowId("editor".into())
}
fn comp() -> CompositionDraft {
    CompositionDraft {
        name: "Main".into(),
        width: 1920.,
        height: 1080.,
        fps: 30.,
        duration: 5.,
        background: "#000000".into(),
    }
}
fn mixed(values: &[f64]) -> MixedNumber {
    MixedNumber::new(values)
}
fn velocity() -> VelocityDraft {
    VelocityDraft {
        incoming_speed: mixed(&[10.]),
        incoming_influence: mixed(&[20.]),
        outgoing_speed: mixed(&[30.]),
        outgoing_influence: mixed(&[40.]),
    }
}
fn fixture() -> (KernelActor, KernelHandle, Context) {
    let actor = KernelActor::spawn(KernelConfig::default());
    let h = actor.handle();
    h.open("P",OpenSource::Untitled(parse(r##"{"id":"P","name":"Project","compId":"C","w":1920,"h":1080,"fps":30,"dur":5,"bg":"#000000","comps":{"C":{"id":"C","name":"Main","w":1920,"h":1080,"fps":30,"dur":5,"bg":"#000000","layers":[{"id":"L","name":"Layer","type":"solid","on":true,"from":0,"dur":5,"d":{"w":200,"h":100,"color":"#ffffff"},"p":{"opacity":{"v":100,"expr":null,"kf":[{"i":"K1","t":0,"v":10},{"i":"K2","t":1,"v":20},{"i":"K3","t":2,"v":30}]}}},{"id":"LOCK","name":"Locked","type":"solid","lock":true,"on":true,"from":0,"dur":5,"d":{"w":100,"h":100,"color":"#ffffff"},"p":{"opacity":{"v":100,"kf":[{"i":"KL","t":0,"v":55}]}}}]},"OTHER":{"id":"OTHER","name":"Other","w":1920,"h":1080,"fps":30,"dur":5,"bg":"#000000","layers":[]}},"opaque":{"extension":"keep"}}"##).unwrap())).wait().unwrap().unwrap();
    let ctx = Context::capture(&h, "P").unwrap();
    (actor, h, ctx)
}
fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}
fn root(h: &KernelHandle) -> Value {
    h.published("P")
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value()
}
fn key<'a>(v: &'a Value, id: &str) -> &'a Value {
    fn find<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
        if v.get("id").and_then(Value::as_str) == Some(id)
            || v.get("i").and_then(Value::as_str) == Some(id)
        {
            return Some(v);
        }
        match v {
            Value::Object(m) => m.iter().find_map(|(_, v)| find(v, id)),
            Value::Array(a) => a.iter().find_map(|v| find(v, id)),
            _ => None,
        }
    }
    find(v, id).unwrap()
}
fn apply(edit: PreparedEdit, h: &KernelHandle, n: u64) {
    assert!(matches!(
        edit.submit(h, n).wait().unwrap().unwrap().reply,
        Reply::Applied { .. }
    ));
}

#[test]
fn composition_copy_and_reading_order() {
    for settings in [false, true] {
        let s = comp().surface(id(), owner(), settings, 2);
        assert_eq!(
            s.title,
            if settings {
                "Composition Settings"
            } else {
                "New Composition"
            }
        );
        assert_eq!(
            s.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
            vec!["Cancel", if settings { "Apply" } else { "Create" }]
        );
        assert_eq!(
            s.fields
                .iter()
                .map(|f| f.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Name",
                "Resolution",
                "Size",
                "Frame Rate",
                "Duration",
                "Background"
            ]
        );
        assert_eq!(s.first_selected, Some("name".into()));
        assert_eq!(s.warning,settings.then(||"2 layers extend past the new duration. They’re kept but won’t play past the end.".into()));
    }
}
#[test]
fn composition_all_validation_bounds_and_nonfinite() {
    let mut c = comp();
    for n in [f64::NAN, f64::INFINITY, 15., 8193.] {
        c.width = n;
        assert_eq!(c.errors().get("size"), Some(&SIZE_ERROR));
        assert!(!c.surface(id(), owner(), false, 0).buttons[1].enabled);
    }
    for n in [16., 8192.] {
        c.width = n;
        c.height = n;
        assert!(!c.errors().contains_key("size"));
    }
    c = comp();
    for n in [0., 241., f64::NAN] {
        c.fps = n;
        assert_eq!(c.errors().get("fps"), Some(&RATE_ERROR));
    }
    for n in [1., 240.] {
        c.fps = n;
        assert!(!c.errors().contains_key("fps"));
    }
    for n in [0., 3601., f64::INFINITY] {
        c.duration = n;
        assert_eq!(c.errors().get("duration"), Some(&DURATION_ERROR));
    }
    for n in [0.1, 3600.] {
        c.duration = n;
        assert!(!c.errors().contains_key("duration"));
    }
    c.name = " \n ".into();
    assert_eq!(c.errors().get("name"), Some(&NAME_ERROR));
}
#[test]
fn composition_names_fill_first_gap() {
    assert_eq!(unique_name("Comp", &ids(&["Comp 1", "Comp 3"])), "Comp 2");
    assert_eq!(unique_name("Pre-comp", &ids(&["Pre-comp 1"])), "Pre-comp 2");
}
#[test]
fn precompose_exact_copy_and_enabled_states() {
    let mut d = PrecomposeDraft {
        name: "Pre-comp 1".into(),
        layer_name: Some("Title".into()),
        can_leave: true,
        leave: false,
        adjust_duration: false,
        open_after: false,
    };
    let s = d.surface(id(), owner());
    assert_eq!(s.title, "Pre-compose");
    assert_eq!(s.fields[1].label, "Leave all attributes in “Title”");
    assert_eq!(s.fields[1].description.as_deref(), Some(LEAVE_DESCRIPTION));
    assert_eq!(s.fields[2].description.as_deref(), Some(MOVE_DESCRIPTION));
    assert!(s.fields[3].enabled);
    assert_eq!(s.fields[4].label, "Open New Composition");
    d.leave = true;
    assert!(!d.surface(id(), owner()).fields[3].enabled);
    d.can_leave = false;
    let s = d.surface(id(), owner());
    assert!(!s.fields[1].enabled);
    assert_eq!(s.fields[1].description.as_deref(), Some(LEAVE_DISABLED));
    assert!(s.fields[3].enabled);
}
#[test]
fn mixed_empty_preserves_component_invalid_gets_field_error() {
    let mut m = mixed(&[1., 2.]);
    assert_eq!(m.parsed(), Ok(None));
    for s in ["NaN", "inf", "not a number"] {
        m.draft = s.into();
        assert_eq!(m.parsed(), Err(NUMBER_ERROR));
        assert!(
            !keyframe_value(id(), owner(), "Opacity", "", 2, &[m.clone()], "%").buttons[1].enabled
        );
    }
    m.draft = "-1.5".into();
    assert_eq!(m.parsed(), Ok(Some(-1.5)));
}
#[test]
fn scale_default_non_numeric_error_and_invalid_states() {
    let mut d = ScaleDraft::default();
    let s = d.surface(id(), owner());
    assert_eq!(
        s.line.as_deref(),
        Some("Scales from the earliest selected keyframe.")
    );
    assert_eq!(s.fields[0].values, vec!["100"]);
    d.numeric = false;
    let s = d.surface(id(), owner());
    assert!(!s.fields[1].enabled);
    assert_eq!(s.fields[1].tooltip, Some(SCALE_TOOLTIP));
    d.collision = true;
    let s = d.surface(id(), owner());
    assert_eq!(s.fields[0].error, Some(COLLISION_ERROR));
    assert!(!s.buttons[1].enabled);
    d.collision = false;
    for n in [0., -1., f64::NAN] {
        d.time_percent = n;
        assert!(!d.valid());
    }
}

#[test]
fn composition_settings_edit_the_composition_they_opened_on_not_the_open_one() {
    let (_actor, h, mut c) = fixture();
    let open = c.composition.clone();
    c.composition = "OTHER".into();
    let mut draft = c.composition_draft(true).unwrap();
    assert_eq!(draft.name, "Other");
    draft.name = "Renamed".into();
    apply(c.composition_edit(&draft, None).unwrap(), &h, 1);
    let comps = root(&h).get("comps").unwrap().clone();
    let name = |id: &str| comps.get(id).unwrap().get("name").unwrap().clone();
    assert_eq!(name("OTHER").as_str(), Some("Renamed"));
    assert_eq!(name(&open).as_str(), Some("Main"));
}

#[test]
fn kernel_composition_settings_one_undo_keeps_coordinates_and_opaque_data() {
    let (_actor, h, c) = fixture();
    let before = root(&h);
    let mut draft = c.composition_draft(true).unwrap();
    draft.name = "Changed".into();
    draft.width = 3840.;
    draft.duration = 1.;
    assert_eq!(c.extending_layers(1.).unwrap(), 2);
    apply(c.composition_edit(&draft, None).unwrap(), &h, 1);
    let p = h.published("P").unwrap();
    assert_eq!(
        p.history.undo_label.as_deref(),
        Some("Composition Settings")
    );
    assert_eq!(root(&h).get("opaque"), before.get("opaque"));
    assert_eq!(key(&root(&h), "L").get("p"), key(&before, "L").get("p"));
    h.call(
        "P",
        Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
    )
    .wait()
    .unwrap()
    .unwrap();
    assert_eq!(root(&h), before);
}
#[test]
fn kernel_new_composition_unique_default_and_create() {
    let (_actor, h, c) = fixture();
    let d = c.composition_draft(false).unwrap();
    assert_eq!(d.name, "Comp 1");
    apply(c.composition_edit(&d, Some("NEW")).unwrap(), &h, 1);
    assert!(root(&h).get("comps").unwrap().get("NEW").is_some());
}
#[test]
fn kernel_key_edits_exclude_locked_and_undo_as_one() {
    let (_actor, h, c) = fixture();
    let before = root(&h);
    assert_eq!(c.keys(&ids(&["KL"])).unwrap().len(), 0);
    apply(
        c.values(&ids(&["K1", "K2", "KL"]), &mixed(&[42.])).unwrap(),
        &h,
        1,
    );
    let after = root(&h);
    assert_eq!(
        key(&after, "K1").get("v").and_then(Value::as_f64),
        Some(42.)
    );
    assert_eq!(
        key(&after, "K2").get("v").and_then(Value::as_f64),
        Some(42.)
    );
    assert_eq!(
        key(&after, "KL").get("v").and_then(Value::as_f64),
        Some(55.)
    );
    h.call(
        "P",
        Request::new(Caller::native(), Call::History(HistoryCall::Undo)),
    )
    .wait()
    .unwrap()
    .unwrap();
    assert_eq!(root(&h), before);
}
#[test]
fn kernel_velocity_sets_bezier_disables_auto_and_sets_each_side() {
    let (_actor, h, c) = fixture();
    apply(c.velocity(&ids(&["K1", "K2"]), &velocity()).unwrap(), &h, 1);
    let r = root(&h);
    let k = key(&r, "K2");
    assert_eq!(k.get("autoBezier"), Some(&Value::Bool(false)));
    assert_eq!(k.get("inInterp").and_then(Value::as_str), Some("bezier"));
    assert_eq!(
        k.get("inEase")
            .unwrap()
            .as_object()
            .unwrap()
            .get("speed")
            .and_then(Value::as_f64),
        Some(10.)
    );
    assert_eq!(
        k.get("outEase")
            .unwrap()
            .as_object()
            .unwrap()
            .get("influence")
            .and_then(Value::as_f64),
        Some(40.)
    );
}
#[test]
fn kernel_scale_refuses_overlap_and_layer_end_without_mutation() {
    let (_actor, h, c) = fixture();
    let before = root(&h);
    for n in [200., 600.] {
        let d = ScaleDraft {
            time_percent: n,
            ..Default::default()
        };
        assert!(matches!(
            c.scale(&ids(&["K1", "K2"]), &d),
            Err(DialogError::Invalid(COLLISION_ERROR))
        ));
        assert_eq!(root(&h), before);
    }
}
#[test]
fn kernel_scale_exact_about_earliest_and_single_undo() {
    let (_actor, h, c) = fixture();
    let d = ScaleDraft {
        time_percent: 150.,
        value_percent: 200.,
        ..Default::default()
    };
    apply(c.scale(&ids(&["K1", "K2"]), &d).unwrap(), &h, 1);
    let r = root(&h);
    assert_eq!(key(&r, "K2").get("t").and_then(Value::as_f64), Some(1.5));
    assert_eq!(key(&r, "K2").get("v").and_then(Value::as_f64), Some(40.));
}

#[test]
fn kernel_velocity_blank_fields_preserve_each_keys_native_ease() {
    let (_actor, h, c) = fixture();
    apply(c.velocity(&ids(&["K1", "K2"]), &velocity()).unwrap(), &h, 1);
    let c = Context::capture(&h, "P").unwrap();
    let before = root(&h);
    let d = VelocityDraft {
        incoming_speed: mixed(&[1., 2.]),
        incoming_influence: mixed(&[1., 2.]),
        outgoing_speed: mixed(&[1., 2.]),
        outgoing_influence: mixed(&[1., 2.]),
    };
    apply(c.velocity(&ids(&["K1", "K2"]), &d).unwrap(), &h, 2);
    let after = root(&h);
    for id in ["K1", "K2"] {
        for side in ["inEase", "outEase"] {
            assert_eq!(key(&after, id).get(side), key(&before, id).get(side));
        }
    }
}

#[test]
fn clearing_a_uniform_value_is_invalid_but_mixed_can_stay_empty() {
    let mut uniform = mixed(&[12.]);
    uniform.draft.clear();
    assert_eq!(uniform.parsed(), Err(NUMBER_ERROR));
    assert_eq!(mixed(&[1., 2.]).parsed(), Ok(None));
}

#[test]
fn velocity_groups_are_part_of_the_inspectable_surface() {
    let s = velocity().surface(id(), owner(), 2, "px");
    assert_eq!(
        s.groups.iter().map(|g| g.title).collect::<Vec<_>>(),
        vec!["Incoming", "Outgoing"]
    );
    assert_eq!(s.groups[0].fields, vec!["in.speed", "in.influence"]);
    assert_eq!(s.groups[1].fields, vec!["out.speed", "out.influence"]);
}

#[test]
fn settings_name_only_keeps_authored_fill_and_work_area() {
    let (_actor, h, _c) = fixture();
    let mut project = root(&h);
    let comp = project
        .as_object_mut()
        .unwrap()
        .get_mut("comps")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("C")
        .unwrap()
        .as_object_mut()
        .unwrap();
    let fill=parse(r##"{"type":"linear","future":"preserve","stops":[{"color":"#000000","position":0},{"color":"#ffffff","position":100}]}"##).unwrap();
    comp.insert("backgroundFill", fill.clone());
    comp.insert("work", parse("[1,3]").unwrap());
    h.close("P").wait().unwrap();
    h.open("P", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let c = Context::capture(&h, "P").unwrap();
    let mut d = c.composition_draft(true).unwrap();
    d.name = "Rename Only".into();
    apply(c.composition_edit(&d, None).unwrap(), &h, 9);
    let p = root(&h);
    let comp = p.get("comps").unwrap().get("C").unwrap();
    assert_eq!(comp.get("backgroundFill"), Some(&fill));
    assert_eq!(comp.get("work"), Some(&parse("[1,3]").unwrap()));
}
#[test]
fn settings_fractional_rate_stays_exact_and_subframe_duration_never_silently_clamps() {
    let (_actor, h, c) = fixture();
    let mut d = c.composition_draft(true).unwrap();
    d.fps = 29.97;
    apply(c.composition_edit(&d, None).unwrap(), &h, 1);
    assert_eq!(
        root(&h)
            .get("comps")
            .unwrap()
            .get("C")
            .unwrap()
            .get("fps")
            .and_then(Value::as_f64),
        Some(29.97)
    );
    let c = Context::capture(&h, "P").unwrap();
    let mut d = c.composition_draft(true).unwrap();
    d.fps = 1.;
    d.duration = 0.1;
    assert!(matches!(
        c.composition_edit(&d, None),
        Err(DialogError::MissingBacking(_))
    ));
}

#[test]
fn legacy_velocity_handles_are_adopted_before_preserving_mixed_components() {
    let (_actor, h, _c) = fixture();
    let mut p = root(&h);
    let k = p
        .as_object_mut()
        .unwrap()
        .get_mut("comps")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("C")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .as_object_mut()
        .unwrap()
        .get_mut("p")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("opacity")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("kf")
        .unwrap()
        .as_array_mut()
        .unwrap();
    k[1].as_object_mut()
        .unwrap()
        .insert("eo", parse("[0.25,0.5]").unwrap());
    k[1].as_object_mut()
        .unwrap()
        .insert("ei", parse("[0.75,0.5]").unwrap());
    h.close("P").wait().unwrap();
    h.open("P", OpenSource::Untitled(p))
        .wait()
        .unwrap()
        .unwrap();
    let c = Context::capture(&h, "P").unwrap();
    let before = c.velocity_keys(&ids(&["K2"])).unwrap().remove(0);
    assert!(before.outgoing.speed > 0.);
    let d = VelocityDraft {
        incoming_speed: mixed(&[1., 2.]),
        incoming_influence: mixed(&[1., 2.]),
        outgoing_speed: mixed(&[1., 2.]),
        outgoing_influence: mixed(&[1., 2.]),
    };
    apply(c.velocity(&ids(&["K2"]), &d).unwrap(), &h, 99);
    let c = Context::capture(&h, "P").unwrap();
    let after = c.velocity_keys(&ids(&["K2"])).unwrap().remove(0);
    assert_eq!(before.incoming, after.incoming);
    assert_eq!(before.outgoing, after.outgoing);
}

#[test]
fn reviewed_precompose_disabled_label_and_scale_errors_are_exact() {
    let d = PrecomposeDraft {
        name: "Pre-comp".into(),
        layer_name: Some("Ignored".into()),
        can_leave: false,
        leave: false,
        adjust_duration: false,
        open_after: false,
    };
    assert_eq!(
        d.surface(id(), owner()).fields[1].label,
        "Leave all attributes in the layer"
    );
    let d = ScaleDraft {
        time_percent: 0.,
        value_percent: f64::NAN,
        ..Default::default()
    };
    let s = d.surface(id(), owner());
    assert_eq!(s.fields[0].error, Some(TIME_PERCENT_ERROR));
    assert_eq!(s.fields[1].error, Some(NUMBER_ERROR));
    assert!(!s.buttons[1].enabled);
}
#[test]
fn the_composition_size_menu_follows_the_dimensions() {
    let mut c = comp();
    assert_eq!(c.resolution_index(), 1);
    assert!(c.choose_resolution("Vertical HD · 1080×1920"));
    assert_eq!((c.width, c.height, c.resolution_index()), (1080., 1920., 3));
    c.width = 1000.;
    assert_eq!(c.resolution_index(), 6, "Custom follows the presets");
    assert!(!c.choose_resolution("Custom"));
    assert_eq!((c.width, c.height), (1000., 1920.));
}
