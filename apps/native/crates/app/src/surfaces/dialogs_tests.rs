use super::Key;
use super::kernel::*;
use super::*;
use pm_kernel::{
    Call, Caller, HistoryCall, KernelActor, KernelConfig, KernelHandle, OpenSource, Reply, Request,
};
use pm_model::{Value, json::parse};

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
fn composition_resolution_menu_and_custom() {
    let mut c = comp();
    for (label, w, h) in RESOLUTIONS {
        assert!(c.choose_resolution(label));
        assert_eq!((c.width, c.height), (*w, *h));
        assert_eq!(c.resolution(), *label);
    }
    c.width = 2000.;
    assert_eq!(c.resolution(), "Custom");
    assert!(!c.choose_resolution("bogus"));
}
#[test]
fn composition_names_fill_first_gap() {
    assert_eq!(unique_name("Comp", &ids(&["Comp 1", "Comp 3"])), "Comp 2");
    assert_eq!(unique_name("Pre-comp", &ids(&["Pre-comp 1"])), "Pre-comp 2");
}
#[test]
fn timecode_input_retains_frame_time_and_refuses_invalid() {
    assert_eq!(duration_input("00:01:02:15", 30.), Some(62.5));
    assert_eq!(duration_input("0.1", 30.), Some(0.1));
    for s in ["00:99:00:00", "00:00:00:30", "1:2", "NaN", "Infinity"] {
        assert_eq!(duration_input(s, 30.), None);
    }
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
fn delete_composition_copy_counts_and_safe_default() {
    for n in [0, 1, 4] {
        let s = delete_composition(id(), owner(), "Title", n);
        assert_eq!(s.title, "Delete “Title”?");
        assert_eq!(
            s.line,
            Some(if n == 0 {
                "You can undo this.".into()
            } else {
                format!(
                    "It’s used in {n} other compositions. Those layers are removed too. You can undo this."
                )
            })
        );
        assert!(s.buttons[0].destructive);
        assert!(!s.buttons[0].default);
        assert_eq!(s.keyboard(Key::Return), KeyEffect::Cancel);
    }
}
#[test]
fn keyframe_copy_single_multiple_and_two_wells() {
    assert_eq!(
        KEYFRAME_MENU,
        &["Edit Value…", "Keyframe Velocity…", "Scale Keyframes…"]
    );
    let s = keyframe_value(
        id(),
        owner(),
        "Position",
        "00:00:01:00",
        1,
        &[mixed(&[1.]), mixed(&[2.])],
        "px",
    );
    assert_eq!(s.title, "Keyframe Value");
    assert_eq!(s.line.as_deref(), Some("Position at 00:00:01:00"));
    assert_eq!(s.fields[0].values, vec!["1", "2"]);
    let s = keyframe_value(id(), owner(), "Opacity", "", 3, &[mixed(&[1., 2.])], "%");
    assert_eq!(s.line.as_deref(), Some("Opacity · 3 keyframes"));
    assert_eq!(s.fields[0].placeholder, Some("Mixed"));
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
fn velocity_exact_copy_units_groups_mixed_and_bounds() {
    let mut d = velocity();
    let s = d.surface(id(), owner(), 3, "px");
    assert_eq!(s.title, "Keyframe Velocity");
    assert_eq!(s.line.as_deref(), Some("3 keyframes"));
    assert_eq!(VELOCITY_GROUPS, &["Incoming", "Outgoing"]);
    assert_eq!(s.fields[0].unit, "px/s");
    assert_eq!(s.fields[1].unit, "%");
    for n in ["0", "100.1", "NaN"] {
        d.incoming_influence.draft = n.into();
        let s = d.surface(id(), owner(), 3, "px");
        assert_eq!(s.fields[0].error, Some(VELOCITY_ERROR));
        assert!(!s.buttons[1].enabled);
    }
    for n in ["0.1", "100", ""] {
        d.incoming_influence.draft = n.into();
        assert!(d.valid());
    }
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
fn expression_copy_buttons_keyboard_and_native_text_config() {
    let mut d = ExpressionDraft::new("value".into(), "Title".into(), "Opacity".into());
    d.preview = Preview::Value("100 %".into());
    let s = d.surface(id(), owner());
    assert_eq!(s.title, "Expression");
    assert_eq!(s.line.as_deref(), Some("Title · Opacity"));
    assert_eq!(s.fields[0].voiceover_label(), "Expression for Opacity");
    assert_eq!(
        s.buttons.iter().map(|b| b.label).collect::<Vec<_>>(),
        vec!["Remove Expression", "Cancel", "Apply"]
    );
    for k in [Key::Return, Key::Tab] {
        assert_eq!(s.keyboard(k), KeyEffect::Type);
    }
    assert_eq!(s.keyboard(Key::CommandReturn), KeyEffect::Apply);
    assert_eq!(s.keyboard(Key::ControlTab), KeyEffect::LeaveEditor);
    assert_eq!(s.keyboard(Key::Escape), KeyEffect::Cancel);
    assert_eq!(EXPRESSION_TEXT.lines, 10);
    assert!(!EXPRESSION_TEXT.smart_quotes);
    assert!(!EXPRESSION_TEXT.smart_dashes);
    assert!(!EXPRESSION_TEXT.substitutions);
    assert!(!EXPRESSION_TEXT.spell_checking);
    assert!(!EXPRESSION_TEXT.syntax_colouring);
    assert!(!EXPRESSION_TEXT.line_numbers);
    assert_eq!(EXPRESSION_TEXT.font, "SF Mono");
    assert_eq!(INSERT_TITLE, "Insert ⌄");
}
#[test]
fn expression_syntax_blocks_runtime_allows_apply() {
    let mut d = ExpressionDraft::new(String::new(), "Title".into(), "Opacity".into());
    for (p, enabled) in [
        (
            Preview::Syntax {
                line: 2,
                message: "Syntax".into(),
            },
            false,
        ),
        (
            Preview::Runtime {
                line: 1,
                message: "Runtime".into(),
            },
            true,
        ),
        (Preview::Value("100 %".into()), true),
        (Preview::Pending, false),
        (Preview::Unavailable, false),
    ] {
        d.preview = p;
        assert_eq!(d.surface(id(), owner()).buttons[1].enabled, enabled);
    }
    assert_eq!(
        Preview::Value("100 %".into()).line().as_deref(),
        Some("At the playhead: 100 %")
    );
    assert_eq!(
        Preview::Runtime {
            line: 3,
            message: "Oops".into()
        }
        .line()
        .as_deref(),
        Some("Line 3: Oops")
    );
}
#[test]
fn expression_debounce_latest_only_and_announcement_change_detection() {
    let now = Instant::now();
    let mut d = ExpressionDraft::new(String::new(), "L".into(), "P".into());
    d.edit("value".into(), now);
    assert!(d.take_job(now + Duration::from_millis(249)).is_none());
    let old = d.take_job(now + Duration::from_millis(250)).unwrap();
    assert!(d.take_job(now + Duration::from_secs(1)).is_none());
    d.edit("time".into(), now + Duration::from_millis(260));
    assert!(!d.complete(&old, Preview::Value("0".into())));
    assert!(d.take_job(now + Duration::from_millis(509)).is_none());
    let job = d.take_job(now + Duration::from_millis(510)).unwrap();
    assert!(d.complete(&job, Preview::Value("1 s".into())));
    assert!(!d.complete(&job, Preview::Value("1 s".into())));
}
#[test]
fn expression_insert_uses_cocoa_utf16_caret_and_preserves_authored_text() {
    let mut d = ExpressionDraft::new("🦀+value".into(), "L".into(), "P".into());
    assert!(!d.insert(1..2, "time", Instant::now()));
    assert!(d.insert(3..8, "time", Instant::now()));
    assert_eq!(d.source, "🦀+time");
}
#[test]
fn preset_exact_alerts_menu_parent_and_accessory() {
    let export = DialogId("export".into());
    for s in [
        replace_preset(id(), owner(), export.clone(), "Film"),
        delete_preset(id(), owner(), export.clone(), "Film"),
        rename_preset(id(), owner(), export.clone(), "Film"),
    ] {
        assert_eq!(s.piece, NativePiece::Alert);
        assert_eq!(s.parent_sheet, Some(export.clone()));
    }
    assert_eq!(
        replace_preset(id(), owner(), export.clone(), "Film").title,
        "A preset named “Film” already exists. Do you want to replace it?"
    );
    let s = delete_preset(id(), owner(), export.clone(), "Film");
    assert_eq!(s.title, "Delete the preset “Film”?");
    assert!(s.buttons[0].destructive);
    assert!(s.buttons[1].default);
    let s = rename_preset(id(), owner(), export, "Film");
    assert_eq!(s.title, "Rename Preset");
    assert_eq!(s.fields[0].values, vec!["Film"]);
    assert_eq!(PRESET_RENAME_ACCESSORY_WIDTH, 240.);
    assert_eq!(
        preset_menu(Some("Film")),
        vec!["", "Rename “Film”…", "Delete “Film”"]
    );
    assert!(preset_menu(None).is_empty());
}
#[test]
fn queue_serializes_per_window_deduplicates_and_restores_focus() {
    let mut q = DialogQueue::default();
    let mut h = HeadlessHost::default();
    let a = delete_composition(id(), owner(), "A", 0);
    let mut b = a.clone();
    b.id = DialogId("B".into());
    assert!(q.present(a.clone(), &mut h));
    assert!(!q.present(a, &mut h));
    assert!(q.present(b.clone(), &mut h));
    assert_eq!(h.events.len(), 1);
    let mut c = b.clone();
    c.id = DialogId("C".into());
    c.owner = WindowId("other".into());
    assert!(q.present(c, &mut h));
    assert_eq!(h.events.len(), 2);
    assert!(q.close(&id(), &mut h));
    assert!(matches!(
        &h.events[2],
        HostEvent::Close {
            restore_trigger: false,
            ..
        }
    ));
    assert!(matches!(&h.events[3],HostEvent::Present(s) if s.id==b.id));
    assert!(q.close(&b.id, &mut h));
    assert!(matches!(
        &h.events[4],
        HostEvent::Close {
            restore_trigger: true,
            ..
        }
    ));
    assert!(!q.close(&b.id, &mut h));
}
#[test]
fn stale_target_closes_with_exact_banner_and_medium_ax() {
    let mut q = DialogQueue::default();
    let mut h = HeadlessHost::default();
    q.present(comp().surface(id(), owner(), true, 0), &mut h);
    assert!(q.stale(&id(), "Composition", &mut h));
    assert!(
        matches!(&h.events[2],HostEvent::Banner {text,announcement:Announcement::Medium,..} if text=="Composition was changed, so the dialog closed.")
    );
}
#[test]
fn sheet_token_references_and_full_keyboard_labels() {
    let s = comp().surface(id(), owner(), false, 0);
    assert_eq!(s.width_token, "metrics.sheet_width");
    assert_eq!(s.inset_token, "metrics.sheet_inset");
    assert_eq!(s.value_column_token, "metrics.sheet_value_column");
    assert!(s.fields.iter().all(|f| !f.voiceover_label().is_empty()));
    assert_eq!(s.keyboard(Key::Tab), KeyEffect::NextInReadingOrder);
    assert_eq!(s.keyboard(Key::Return), KeyEffect::Apply);
    assert_eq!(s.keyboard(Key::Escape), KeyEffect::Cancel);
    assert!(
        DESIGN_TODOS
            .iter()
            .all(|s| s.starts_with("DESIGN-TODO(opus):"))
    );
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
fn kernel_blank_components_leave_values_unchanged() {
    let (_actor, h, c) = fixture();
    let before = root(&h);
    apply(
        c.component_values(&[
            (ids(&["K1"]), mixed(&[1., 2.])),
            (ids(&["K2"]), mixed(&[99.])),
        ])
        .unwrap(),
        &h,
        1,
    );
    assert_eq!(key(&root(&h), "K1").get("v"), key(&before, "K1").get("v"));
    assert_eq!(
        key(&root(&h), "K2").get("v").and_then(Value::as_f64),
        Some(99.)
    );
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
fn kernel_expression_preserves_source_and_remove_undo() {
    let (_actor, h, c) = fixture();
    let source = "value + time";
    apply(
        c.expression("L", "opacity", Some(source.into())).unwrap(),
        &h,
        1,
    );
    assert_eq!(
        key(&root(&h), "L")
            .get("p")
            .unwrap()
            .get("opacity")
            .unwrap()
            .get("expr")
            .and_then(Value::as_str),
        Some(source)
    );
    let c = Context::capture(&h, "P").unwrap();
    apply(c.expression("L", "opacity", None).unwrap(), &h, 2);
    assert_eq!(
        key(&root(&h), "L")
            .get("p")
            .unwrap()
            .get("opacity")
            .unwrap()
            .get("expr"),
        Some(&Value::Null)
    );
}
#[test]
fn kernel_invalid_expression_never_applies() {
    let (_actor, h, c) = fixture();
    let before = root(&h);
    assert!(
        c.expression("L", "opacity", Some("value + (".into()))
            .is_err()
    );
    assert_eq!(root(&h), before);
}
#[test]
fn kernel_fences_revision_and_reopened_document_session() {
    let (_actor, h, c) = fixture();
    let first = c.values(&ids(&["K1"]), &mixed(&[42.])).unwrap();
    let stale = c.values(&ids(&["K2"]), &mixed(&[43.])).unwrap();
    apply(first, &h, 1);
    assert!(matches!(
        stale.submit(&h, 2).wait().unwrap().unwrap().reply,
        Reply::Throw(_)
    ));
    let fresh = Context::capture(&h, "P").unwrap();
    let stale = fresh.values(&ids(&["K1"]), &mixed(&[44.])).unwrap();
    let saved = root(&h);
    assert!(h.close("P").wait().unwrap());
    h.open("P", OpenSource::Untitled(saved.clone()))
        .wait()
        .unwrap()
        .unwrap();
    assert!(!fresh.current(&h));
    assert!(matches!(
        stale.submit(&h, 3).wait().unwrap().unwrap().reply,
        Reply::Throw(_)
    ));
    assert_eq!(root(&h), saved);
}
#[test]
fn kernel_precompose_missing_backing_is_explicit() {
    let (_actor, _h, c) = fixture();
    assert!(matches!(
        c.precompose(&PrecomposeDraft {
            name: "Pre-comp 1".into(),
            layer_name: Some("L".into()),
            can_leave: true,
            leave: false,
            adjust_duration: false,
            open_after: false
        }),
        Err(DialogError::MissingBacking(_))
    ));
}

#[derive(Default)]
struct Archive {
    entries: Vec<Value>,
    fail: bool,
}
impl PresetArchive for Archive {
    fn archive(&mut self, e: &Value) -> Result<(), ServiceCode> {
        if self.fail {
            return Err(ServiceCode::DiskFull);
        }
        self.entries.push(e.clone());
        Ok(())
    }
}
fn preset_fixture(h: &KernelHandle) -> Presets {
    let p = Presets::capture(h).unwrap();
    let p = p
        .save(
            "one",
            "Film",
            parse(r#"{"name":"output","format":"mp4","unknown":{"preserve":true}}"#).unwrap(),
            false,
            &mut Archive::default(),
        )
        .unwrap();
    p.submit(h).wait().unwrap().unwrap();
    Presets::capture(h).unwrap()
}
#[test]
fn preset_kernel_save_select_excludes_output_name_retains_unknown_fields() {
    let (_actor, h, _) = fixture();
    let p = preset_fixture(&h);
    let o = p.select("one").unwrap();
    assert!(o.get("name").is_none());
    assert!(o.get("unknown").is_some());
    assert!(p.select("missing").is_none());
}
#[test]
fn preset_replace_requires_confirmation_archives_full_old_entry() {
    let (_actor, h, _) = fixture();
    let p = preset_fixture(&h);
    let mut archive = Archive::default();
    assert!(matches!(
        p.save("two", "Film", parse("{}").unwrap(), false, &mut archive),
        Err(PresetError::NeedsReplace)
    ));
    assert!(archive.entries.is_empty());
    let p = p
        .save("two", "Film", parse("{}").unwrap(), true, &mut archive)
        .unwrap();
    assert_eq!(archive.entries.len(), 1);
    assert!(
        archive.entries[0]
            .get("options")
            .unwrap()
            .get("unknown")
            .is_some()
    );
    assert_eq!(p.entries.len(), 1);
    p.submit(&h).wait().unwrap().unwrap();
    assert!(Presets::capture(&h).unwrap().select("two").is_some());
}
#[test]
fn preset_delete_archive_failure_keeps_every_byte_and_exact_error() {
    let (_actor, h, _) = fixture();
    let p = preset_fixture(&h);
    let before = p.entries.clone();
    let error = p
        .delete(
            "one",
            &mut Archive {
                fail: true,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(
        error.inline(),
        Some((
            "Couldn’t save the preset.",
            "Free some space and try again."
        ))
    );
    assert_eq!(Presets::capture(&h).unwrap().entries, before);
}
#[test]
fn preset_delete_archives_and_rename_preserves_payload() {
    let (_actor, h, _) = fixture();
    let p = preset_fixture(&h);
    let renamed = p.rename("one", "New").unwrap();
    assert_eq!(
        renamed.entries[0].get("options"),
        p.entries[0].get("options")
    );
    renamed.submit(&h).wait().unwrap().unwrap();
    let p = Presets::capture(&h).unwrap();
    let mut archive = Archive::default();
    p.delete("one", &mut archive)
        .unwrap()
        .submit(&h)
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(archive.entries, p.entries);
    assert!(Presets::capture(&h).unwrap().entries.is_empty());
}
#[test]
fn preset_state_race_refuses_overwrite() {
    let (_actor, h, _) = fixture();
    let p = preset_fixture(&h);
    let a = p.rename("one", "A").unwrap();
    let b = p.rename("one", "B").unwrap();
    a.submit(&h).wait().unwrap().unwrap();
    assert!(b.submit(&h).wait().unwrap().is_err());
    assert_eq!(
        Presets::capture(&h).unwrap().entries[0]
            .get("name")
            .and_then(Value::as_str),
        Some("A")
    );
}
#[test]
fn preset_rename_missing_and_unresolved_validation() {
    let (_actor, h, _) = fixture();
    let p = preset_fixture(&h);
    assert!(matches!(
        p.rename("missing", "New"),
        Err(PresetError::Missing)
    ));
    assert!(matches!(
        p.rename("one", "  "),
        Err(PresetError::DesignTodo)
    ));
}
#[test]
fn all_service_codes_use_approved_informative_copy() {
    for code in [
        ServiceCode::Offline,
        ServiceCode::NotFound,
        ServiceCode::TooLarge,
        ServiceCode::InvalidProject,
        ServiceCode::PermissionDenied,
        ServiceCode::DiskFull,
        ServiceCode::InvalidInput,
        ServiceCode::InvalidData,
        ServiceCode::Unavailable,
        ServiceCode::Unknown,
    ] {
        let (message, info) = PresetError::Service(code).inline().unwrap();
        assert_eq!(message, PRESET_FAILURE);
        assert!(!info.contains("Help ▸"));
    }
    assert_eq!(
        ServiceCode::Unknown.informative(),
        "Try again. If it keeps happening, choose Window ▸ Error Log for details."
    );
}
