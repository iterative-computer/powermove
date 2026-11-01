use pm_kernel::commands::layers::LayerPatch;
use pm_kernel::{
    Call, Caller, Command, HistoryCall, KernelActor, KernelConfig, KernelHandle, OpenSource, Reply,
    Request, SelectionCall,
};
use pm_model::{Value, json::parse};
use pm_panel_inspector::{input::*, *};

struct Fixture {
    actor: Option<KernelActor>,
    handle: KernelHandle,
    model: Inspector,
}
impl Fixture {
    fn new() -> Self {
        let actor = KernelActor::spawn(KernelConfig::default());
        let handle = actor.handle();
        let mut project=parse(r##"{"id":"P","name":"Fixture","compId":"C","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{"img":{"id":"img","kind":"image","name":"Synthetic image"}},"comps":{},"layers":[
   {"id":"a","type":"shape","name":"A","from":1,"dur":5,"on":true,"lock":false,"scaleLinked":false,"p":{"position.x":{"v":10,"kf":[],"expr":null},"position.y":{"v":20,"kf":[],"expr":null},"scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":50,"kf":[],"expr":null},"opacity":{"v":100,"kf":[],"expr":null}},"d":{"color":"#ff0000","w":100,"h":100,"shape":"rect","radius":10,"text":"private"},"fx":[{"id":"f1","type":"blur","on":true,"open":true,"p":{"amount":{"v":5,"kf":[],"expr":null}}},{"id":"f2","type":"blur","on":true,"p":{"amount":{"v":6,"kf":[],"expr":null}}},{"id":"f3","type":"other","on":false,"p":{"amount":{"v":7,"kf":[],"expr":null}}}],"masks":[]},
   {"id":"b","type":"text","name":"B","from":0,"dur":10,"on":true,"lock":false,"p":{"position.x":{"v":30,"kf":[],"expr":null},"scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":100,"kf":[],"expr":null},"opacity":{"v":100,"kf":[],"expr":null}},"d":{"color":"#00ff00","w":200,"text":"original","size":64},"fx":[{"id":"b1","type":"blur","on":true,"p":{"amount":{"v":1,"kf":[],"expr":null}}}],"masks":[]},
   {"id":"im","type":"image","name":"Image","from":0,"dur":10,"on":true,"d":{"w":200,"h":100},"p":{},"fx":[],"masks":[]},
   {"id":"g","type":"group","name":"G","from":0,"dur":10,"on":true,"lock":true,"p":{"opacity":{"v":100,"kf":[],"expr":null}},"fx":[],"d":{},"masks":[]},
   {"id":"child","type":"shape","name":"Child","group":"g","from":0,"dur":10,"on":true,"p":{"opacity":{"v":100,"kf":[],"expr":null}},"d":{"color":"#0000ff","w":100},"fx":[],"masks":[]}
  ]}"##).unwrap();
        for layer in project
            .as_object_mut()
            .unwrap()
            .get_mut("layers")
            .unwrap()
            .as_array_mut()
            .unwrap()
        {
            let p = layer
                .as_object_mut()
                .unwrap()
                .get_mut("p")
                .unwrap()
                .as_object_mut()
                .unwrap();
            for key in [
                "position.x",
                "position.y",
                "position.z",
                "anchor.x",
                "anchor.y",
                "anchor.z",
                "scale.x",
                "scale.y",
                "scale.z",
                "orientation.x",
                "orientation.y",
                "orientation.z",
                "rotation",
                "rotation.x",
                "rotation.y",
                "skew",
                "perspective",
                "opacity",
            ] {
                if p.get(key).is_none() {
                    let value = if key.starts_with("scale.") || key == "opacity" {
                        100.0
                    } else if key == "perspective" {
                        50.0
                    } else {
                        0.0
                    };
                    let mut channel = pm_model::Map::new();
                    channel.insert("v", value.into());
                    channel.insert("kf", Value::Array(vec![]));
                    channel.insert("expr", Value::Null);
                    p.insert(key, Value::Object(channel));
                }
            }
        }
        handle
            .open("P", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        let mut model = Inspector::new(handle.published("P").unwrap(), EffectClipboard::default());
        model.register_effect(definition("blur"));
        Self {
            actor: Some(actor),
            handle,
            model,
        }
    }
    fn run(&mut self, request: Request) -> pm_kernel::Ack {
        let ack = self
            .model
            .submit(&self.handle, request)
            .wait()
            .unwrap()
            .unwrap();
        assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
        self.sync();
        ack
    }
    fn sync(&mut self) {
        self.model.sync(self.handle.published("P").unwrap());
    }
    fn select(&mut self, ids: &[&str]) {
        self.run(Request::new(
            Caller::native(),
            SelectionCall::Layers {
                ids: ids.iter().map(|s| s.to_string()).collect(),
                add: false,
            },
        ));
    }
    fn set(&mut self, b: Binding, value: impl Into<Value>) {
        let r = self
            .model
            .set(Caller::native(), b, value.into(), 2.0)
            .unwrap();
        self.run(r);
    }
    fn value(&mut self, layer: &str, path: &str) -> Value {
        self.model
            .binding_state(&prop(layer, path), 2.0)
            .unwrap()
            .value
    }
    fn undo(&mut self) {
        self.run(Request::new(
            Caller::native(),
            Call::History(HistoryCall::Undo),
        ));
    }
    fn redo(&mut self) {
        self.run(Request::new(
            Caller::native(),
            Call::History(HistoryCall::Redo),
        ));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.actor.take().unwrap().shutdown();
    }
}
fn prop(l: &str, p: &str) -> Binding {
    Binding::Property(PropertyId::new(l, p))
}
fn content(l: &str, p: &str) -> Binding {
    Binding::Content {
        layer: l.into(),
        field: p.into(),
    }
}
fn layer(l: &str, p: &str) -> Binding {
    Binding::Layer {
        layer: l.into(),
        field: p.into(),
    }
}
fn definition(t: &str) -> EffectDefinition {
    EffectDefinition {
        effect_type: t.into(),
        version: 1,
        label: t.into(),
        params: vec![ParamDefinition {
            key: "amount".into(),
            kind: "number".into(),
            default: 3.0.into(),
            min: Some(0.0),
            max: Some(100.0),
            reference: false,
        }],
        source_processing: false,
        backdrop: false,
    }
}
fn act(f: &mut Fixture, l: &str, p: &str, a: PropertyAction) {
    let r = f
        .model
        .property_action(Caller::native(), &[PropertyId::new(l, p)], a, 2.0)
        .unwrap();
    f.run(r);
}

#[test]
fn empty_selection_is_composition_and_primary_is_first() {
    let mut f = Fixture::new();
    assert!(f.model.composition_mode());
    f.select(&["b", "a"]);
    assert_eq!(f.model.primary(), Some("b"));
    assert_eq!(f.model.selected_layers(), ["b", "a"]);
}
#[test]
fn selected_group_owns_children() {
    let mut f = Fixture::new();
    f.select(&["child", "g", "a"]);
    assert_eq!(f.model.selected_layers(), ["g", "a"]);
}
#[test]
fn locked_ancestor_skips_child_and_undo_restores_sibling() {
    let mut f = Fixture::new();
    f.select(&["a", "child"]);
    f.set(prop("a", "opacity"), 20.0);
    assert_eq!(f.value("a", "opacity"), 20.0.into());
    assert_eq!(f.value("child", "opacity"), 100.0.into());
    f.undo();
    assert_eq!(f.value("a", "opacity"), 100.0.into());
    f.redo();
    assert_eq!(f.value("a", "opacity"), 20.0.into());
}
#[test]
fn mixed_values_come_from_editable_applicable_targets() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    let state = f.model.binding_state(&content("a", "color"), 2.0).unwrap();
    assert!(state.mixed);
    assert_eq!(state.editable_targets.len(), 2);
    f.set(content("a", "color"), "#ffffff");
    assert!(
        !f.model
            .binding_state(&content("a", "color"), 2.0)
            .unwrap()
            .mixed
    );
}
#[test]
fn content_color_fans_out_across_types() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    f.set(content("a", "color"), "#ffffff");
    assert_eq!(f.value("b", "c.color"), "#ffffff".into());
    f.undo();
    assert_eq!(f.value("b", "c.color"), "#00ff00".into());
}
#[test]
fn type_private_content_does_not_fan_out() {
    let mut f = Fixture::new();
    f.select(&["b", "a"]);
    f.set(content("b", "text"), "changed");
    assert_eq!(
        f.model.layer("a").unwrap().get("d").unwrap().get("text"),
        Some(&Value::from("private"))
    );
}
#[test]
fn dimensions_fan_out_across_types() {
    let mut f = Fixture::new();
    f.select(&["a", "im"]);
    f.set(content("a", "w"), 123.0);
    assert_eq!(f.value("im", "c.w"), 123.0.into());
}
#[test]
fn typed_multiedit_sets_absolute_values() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    f.set(prop("a", "position.x"), 80.0);
    assert_eq!(f.value("b", "position.x"), 80.0.into());
}
#[test]
fn drag_offsets_fixed_targets_from_start_and_is_one_undo() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    let (mut session, r) = f
        .model
        .begin_edit(Caller::native(), &[prop("a", "position.x")], 2.0)
        .unwrap();
    f.run(r);
    for n in [15.0, 22.0] {
        let r = f
            .model
            .edit_step(Caller::native(), &mut session, &[n.into()])
            .unwrap();
        f.run(r);
    }
    f.run(f.model.finish_edit(Caller::native(), false));
    assert_eq!(f.value("a", "position.x"), 22.0.into());
    assert_eq!(f.value("b", "position.x"), 42.0.into());
    f.undo();
    assert_eq!(f.value("a", "position.x"), 10.0.into());
    assert_eq!(f.value("b", "position.x"), 30.0.into());
}
#[test]
fn selection_change_does_not_retarget_live_drag() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    let (mut s, r) = f
        .model
        .begin_edit(Caller::native(), &[prop("a", "position.x")], 2.0)
        .unwrap();
    f.run(r);
    f.select(&["child"]);
    let r = f
        .model
        .edit_step(Caller::native(), &mut s, &[20.0.into()])
        .unwrap();
    f.run(r);
    f.run(f.model.finish_edit(Caller::native(), false));
    assert_eq!(f.value("b", "position.x"), 40.0.into());
}
#[test]
fn escape_cancels_live_edit() {
    let mut f = Fixture::new();
    let (mut s, r) = f
        .model
        .begin_edit(Caller::native(), &[content("b", "text")], 2.0)
        .unwrap();
    f.run(r);
    let r = f
        .model
        .edit_step(Caller::native(), &mut s, &["draft".into()])
        .unwrap();
    f.run(r);
    f.run(f.model.finish_edit(Caller::native(), true));
    assert_eq!(f.value("b", "c.text"), "original".into());
}
#[test]
fn stale_agent_plan_is_refused_atomically() {
    let mut f = Fixture::new();
    let r = f
        .model
        .set(
            Caller::realm(0, 1, pm_kernel::Origin::Mcp),
            prop("a", "opacity"),
            60.0.into(),
            2.0,
        )
        .unwrap();
    f.set(prop("b", "opacity"), 50.0);
    let ack = f.handle.call_blocking("P", r).unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    f.sync();
    assert_eq!(f.value("a", "opacity"), 100.0.into());
}
#[test]
fn linked_scale_preserves_ratio_and_keys_both_axes() {
    let mut f = Fixture::new();
    f.set(layer("a", "scaleLinked"), true);
    f.set(prop("a", "scale.x"), 200.0);
    assert_eq!(f.value("a", "scale.y"), 100.0.into());
    act(&mut f, "a", "scale.x", PropertyAction::Stopwatch);
    act(&mut f, "a", "scale.y", PropertyAction::Stopwatch);
    f.set(prop("a", "scale.x"), 300.0);
    assert_eq!(f.value("a", "scale.y"), 150.0.into());
}
#[test]
fn unlinked_scale_changes_only_chosen_axis() {
    let mut f = Fixture::new();
    f.set(prop("a", "scale.x"), 200.0);
    assert_eq!(f.value("a", "scale.y"), 50.0.into());
}
#[test]
fn stopwatch_adds_then_removes_animation_preserving_shown_value() {
    let mut f = Fixture::new();
    act(&mut f, "a", "opacity", PropertyAction::Stopwatch);
    assert!(
        f.model
            .binding_state(&prop("a", "opacity"), 2.0)
            .unwrap()
            .animated
    );
    f.set(prop("a", "opacity"), 60.0);
    act(&mut f, "a", "opacity", PropertyAction::Stopwatch);
    let s = f.model.binding_state(&prop("a", "opacity"), 2.0).unwrap();
    assert!(!s.animated);
    assert_eq!(s.value, 60.0.into());
}
#[test]
fn diamond_only_removes_current_key() {
    let mut f = Fixture::new();
    act(&mut f, "a", "opacity", PropertyAction::Diamond);
    let r = f
        .model
        .property_action(
            Caller::native(),
            &[PropertyId::new("a", "opacity")],
            PropertyAction::AddKey,
            3.0,
        )
        .unwrap();
    f.run(r);
    act(&mut f, "a", "opacity", PropertyAction::Diamond);
    let s = f.model.binding_state(&prop("a", "opacity"), 2.0).unwrap();
    assert!(s.animated);
    assert!(!s.key_at_playhead);
    assert!(
        f.model
            .binding_state(&prop("a", "opacity"), 3.0)
            .unwrap()
            .key_at_playhead
    );
}
#[test]
fn diamonds_across_selection_add_missing_keys_before_removing_all() {
    let mut f = Fixture::new();
    act(&mut f, "a", "opacity", PropertyAction::Diamond);
    f.select(&["a", "b"]);
    act(&mut f, "a", "opacity", PropertyAction::Diamond);
    assert!(
        f.model
            .binding_state(&prop("a", "opacity"), 2.0)
            .unwrap()
            .key_at_playhead
    );
    assert!(
        f.model
            .binding_state(&prop("b", "opacity"), 2.0)
            .unwrap()
            .key_at_playhead
    );
    act(&mut f, "a", "opacity", PropertyAction::Diamond);
    assert!(
        !f.model
            .binding_state(&prop("b", "opacity"), 2.0)
            .unwrap()
            .key_at_playhead
    );
}
#[test]
fn expression_apply_remove_and_invalid_are_atomic() {
    let mut f = Fixture::new();
    act(
        &mut f,
        "a",
        "opacity",
        PropertyAction::Expression(Some(" value / 2 ".into())),
    );
    assert_eq!(f.value("a", "opacity"), 50.0.into());
    let r = f
        .model
        .property_action(
            Caller::native(),
            &[PropertyId::new("a", "opacity")],
            PropertyAction::Expression(Some("bad(".into())),
            2.0,
        )
        .unwrap();
    let ack = f.handle.call_blocking("P", r).unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    f.sync();
    assert_eq!(f.value("a", "opacity"), 50.0.into());
    act(&mut f, "a", "opacity", PropertyAction::Expression(None));
    assert_eq!(f.value("a", "opacity"), 100.0.into());
}
#[test]
fn reset_animated_effect_uses_definition_default() {
    let mut f = Fixture::new();
    act(&mut f, "a", "f1.amount", PropertyAction::Diamond);
    act(
        &mut f,
        "a",
        "f1.amount",
        PropertyAction::Expression(Some("value+1".into())),
    );
    act(&mut f, "a", "f1.amount", PropertyAction::Reset(None));
    let s = f.model.binding_state(&prop("a", "f1.amount"), 2.0).unwrap();
    assert_eq!(s.value, 3.0.into());
    assert!(!s.animated);
    assert_eq!(s.expression, None);
    f.undo();
    assert!(
        f.model
            .binding_state(&prop("a", "f1.amount"), 2.0)
            .unwrap()
            .animated
    );
}
#[test]
fn transform_reset_uses_composition_center() {
    let mut f = Fixture::new();
    act(&mut f, "a", "position.x", PropertyAction::Reset(None));
    assert_eq!(f.value("a", "position.x"), 320.0.into());
}
#[test]
fn effect_paths_translate_by_stack_position_and_type() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    f.set(prop("a", "f1.amount"), 20.0);
    assert_eq!(f.value("b", "b1.amount"), 20.0.into());
    f.set(prop("a", "f3.amount"), 9.0);
    assert_eq!(f.value("b", "b1.amount"), 20.0.into());
}
#[test]
fn composition_fields_use_kernel_command_and_undo() {
    let mut f = Fixture::new();
    f.set(Binding::Composition("width".into()), 800.0);
    assert_eq!(
        f.model
            .binding_state(&Binding::Composition("width".into()), 0.0)
            .unwrap()
            .value,
        800.0.into()
    );
    f.undo();
    assert_eq!(
        f.model
            .binding_state(&Binding::Composition("width".into()), 0.0)
            .unwrap()
            .value,
        640.0.into()
    );
}
#[test]
fn property_selection_is_not_reveal_or_history() {
    let mut f = Fixture::new();
    let rev = f.model.revision();
    f.run(
        f.model
            .select_channel(Caller::native(), Some("opacity".into())),
    );
    assert_eq!(f.model.revision(), rev);
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn effect_row_selection_never_toggles_disclosure() {
    let mut f = Fixture::new();
    f.model.set_effect_disclosure("a", "f1", false).unwrap();
    f.model
        .select_effect("a", "f1", Modifiers::default())
        .unwrap();
    assert!(!f.model.effect_disclosure("a", "f1").unwrap());
    assert_eq!(f.model.effects.ids, ["f1"]);
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn effect_shift_range_and_command_toggle() {
    let mut f = Fixture::new();
    f.model
        .select_effect("a", "f1", Modifiers::default())
        .unwrap();
    f.model
        .select_effect(
            "a",
            "f3",
            Modifiers {
                shift: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(f.model.effects.ids, ["f1", "f2", "f3"]);
    f.model
        .select_effect(
            "a",
            "f2",
            Modifiers {
                meta: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(f.model.effects.ids, ["f1", "f3"]);
}
#[test]
fn outside_clears_but_nested_effect_controls_preserve_selection() {
    let mut f = Fixture::new();
    f.model
        .select_effect("a", "f1", Modifiers::default())
        .unwrap();
    f.model.effects.pointer_outside(true);
    assert_eq!(f.model.effects.ids, ["f1"]);
    f.model.effects.pointer_outside(false);
    assert!(f.model.effects.ids.is_empty());
}
#[test]
fn toggle_effect_is_undoable_set_effect() {
    let mut f = Fixture::new();
    let r = f
        .model
        .toggle_effect(Caller::native(), "a", "f1", 2.0)
        .unwrap();
    f.run(r);
    assert_eq!(
        f.model
            .layer("a")
            .unwrap()
            .get("fx")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .get("on"),
        Some(&false.into())
    );
    f.undo();
    assert_eq!(
        f.model
            .layer("a")
            .unwrap()
            .get("fx")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .get("on"),
        Some(&true.into())
    );
}
#[test]
fn effect_drop_slots_work_in_both_directions_and_undo() {
    let mut f = Fixture::new();
    let r = f.model.drop_effect(Caller::native(), "a", "f1", 3).unwrap();
    f.run(r);
    assert_eq!(f.model.effect_ids("a").unwrap(), ["f2", "f3", "f1"]);
    let r = f.model.drop_effect(Caller::native(), "a", "f1", 0).unwrap();
    f.run(r);
    assert_eq!(f.model.effect_ids("a").unwrap(), ["f1", "f2", "f3"]);
    f.undo();
    assert_eq!(f.model.effect_ids("a").unwrap(), ["f2", "f3", "f1"]);
}
#[test]
fn effect_delete_selects_nearest_survivor_after_ack() {
    let mut f = Fixture::new();
    f.model
        .select_effect("a", "f2", Modifiers::default())
        .unwrap();
    let (r, next) = f.model.delete_effects(Caller::native(), "a", "f2").unwrap();
    assert_eq!(f.model.effects.ids, ["f2"]);
    f.run(r);
    f.model.effects = next;
    assert_eq!(f.model.effects.ids, ["f3"]);
    assert_eq!(f.model.effect_ids("a").unwrap(), ["f1", "f3"]);
    f.undo();
    assert_eq!(f.model.effect_ids("a").unwrap(), ["f1", "f2", "f3"]);
}
#[test]
fn copy_survives_new_destination_and_paste_has_new_ids() {
    let mut f = Fixture::new();
    f.model.copy_effects("a", "f1").unwrap();
    f.select(&["b"]);
    let (r, next) = f.model.paste_effects(Caller::native(), "b").unwrap();
    f.run(r);
    f.model.effects = next;
    let ids = f.model.effect_ids("b").unwrap();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[1], "f1");
    assert_eq!(f.value("b", &format!("{}.amount", ids[1])), 5.0.into());
    assert_eq!(f.model.clipboard.len(), 1);
    f.undo();
    assert_eq!(f.model.effect_ids("b").unwrap(), ["b1"]);
}
#[test]
fn copy_selected_effects_preserves_stack_order() {
    let mut f = Fixture::new();
    f.model
        .select_effect("a", "f2", Modifiers::default())
        .unwrap();
    f.model
        .select_effect(
            "a",
            "f1",
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(f.model.copy_effects("a", "f2").unwrap(), 2);
    let (r, _) = f.model.paste_effects(Caller::native(), "b").unwrap();
    f.run(r);
    let ids = f.model.effect_ids("b").unwrap();
    assert_eq!(f.value("b", &format!("{}.amount", ids[1])), 5.0.into());
    assert_eq!(f.value("b", &format!("{}.amount", ids[2])), 6.0.into());
}
#[test]
fn missing_effect_definitions_are_retained_and_paste_skips_unavailable() {
    let mut f = Fixture::new();
    f.model.copy_effects("a", "f3").unwrap();
    assert!(matches!(
        f.model.paste_effects(Caller::native(), "b"),
        Err(InspectorError::EmptyEdit)
    ));
    assert_eq!(f.model.effect_ids("a").unwrap().len(), 3);
}
#[test]
fn shared_clipboard_outlives_panel_remount() {
    let mut f = Fixture::new();
    f.model.copy_effects("a", "f1").unwrap();
    let other = Inspector::new(f.handle.published("P").unwrap(), f.model.clipboard.clone());
    assert_eq!(other.clipboard.len(), 1);
}
#[test]
fn property_id_roundtrip_handles_punctuation_unicode() {
    let id = PropertyId::new("layer:/雪", "g.path.é.fill");
    assert_eq!(PropertyId::from_value(&id.value()).unwrap(), id);
    assert!(PropertyId::from_value(&Value::Null).is_err());
}
#[test]
fn window_is_bounded_and_row_ids_survive_values() {
    let mut f = Fixture::new();
    let a = f.model.property_window("a", 0..3, 2.0).unwrap();
    assert_eq!(a.rows.len(), 3);
    f.set(prop("a", "opacity"), 50.0);
    let b = f.model.property_window("a", 0..3, 2.0).unwrap();
    assert_eq!(
        a.rows.iter().map(|r| &r.id).collect::<Vec<_>>(),
        b.rows.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
    assert!(
        f.model
            .property_window("a", usize::MAX..usize::MAX, 2.0)
            .unwrap()
            .rows
            .is_empty()
    );
}
#[test]
fn invalid_inputs_fail_without_mutation() {
    let mut f = Fixture::new();
    assert!(
        f.model
            .set(
                Caller::native(),
                prop("missing", "opacity"),
                1.0.into(),
                0.0
            )
            .is_err()
    );
    assert!(
        f.model
            .set(Caller::native(), prop("a", "opacity"), 1.0.into(), f64::NAN)
            .is_err()
    );
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn add_effect_uses_definition_defaults() {
    let mut f = Fixture::new();
    let r = f.model.add_effect(Caller::native(), "b", "blur").unwrap();
    f.run(r);
    let ids = f.model.effect_ids("b").unwrap();
    assert_eq!(f.value("b", &format!("{}.amount", ids[1])), 3.0.into());
}
#[test]
fn raw_typed_command_remains_atomic_and_caller_keeps_origin() {
    let mut f = Fixture::new();
    let caller = Caller::realm(7, 1, pm_kernel::Origin::Mcp);
    let r = f
        .model
        .commands(
            caller,
            vec![Command::Layer(pm_kernel::LayerCommand::Set {
                layer_id: "b".into(),
                patch: LayerPatch {
                    solo: Some(true),
                    ..Default::default()
                },
            })],
        )
        .unwrap();
    assert_eq!(r.caller, caller);
    f.run(r);
    assert_eq!(f.model.layer("b").unwrap().get("solo"), Some(&true.into()));
}

#[test]
fn multiple_selected_keys_offset_from_captured_value() {
    let mut f = Fixture::new();
    let keys = vec![
        pm_kernel::commands::keyframes::KeyframeSpec::new(0.0, 10.0),
        pm_kernel::commands::keyframes::KeyframeSpec::new(2.0, 30.0),
    ];
    let r = f
        .model
        .commands(
            Caller::native(),
            vec![
                pm_kernel::KeyframeCommand::Replace(Box::new(
                    pm_kernel::commands::keyframes::ReplaceKeyframes {
                        layer_id: "a".into(),
                        channel: "position.x".into(),
                        keyframes: keys,
                        replace: true,
                        expression: None,
                        preserve_hand_edits: false,
                        time: 2.0,
                    },
                ))
                .into(),
            ],
        )
        .unwrap();
    f.run(r);
    let ids = f
        .model
        .layer("a")
        .unwrap()
        .get("p")
        .unwrap()
        .get("position.x")
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.get("i").unwrap().as_str().unwrap().to_owned())
        .collect();
    f.run(Request::new(Caller::native(), SelectionCall::Keys(ids)));
    let base = f.value("a", "position.x").as_f64().unwrap();
    let (mut session, r) = f
        .model
        .begin_edit(Caller::native(), &[prop("a", "position.x")], 2.0)
        .unwrap();
    f.run(r);
    for delta in [5.0, 10.0] {
        let r = f
            .model
            .edit_step(Caller::native(), &mut session, &[(base + delta).into()])
            .unwrap();
        f.run(r);
    }
    f.run(f.model.finish_edit(Caller::native(), false));
    let layer = f.model.layer("a").unwrap();
    let keys = layer
        .get("p")
        .unwrap()
        .get("position.x")
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(keys[0].get("v"), Some(&20.0.into()));
    assert_eq!(keys[1].get("v"), Some(&40.0.into()));
    f.undo();
    assert_eq!(f.value("a", "position.x"), base.into());
}
#[test]
fn unlinked_scale_animation_still_keys_both_axes() {
    let mut f = Fixture::new();
    act(&mut f, "a", "scale.x", PropertyAction::Stopwatch);
    f.set(prop("a", "scale.x"), 150.0);
    assert_eq!(f.value("a", "scale.y"), 50.0.into());
    assert!(
        f.model
            .binding_state(&prop("a", "scale.y"), 2.0)
            .unwrap()
            .key_at_playhead
    );
}
#[test]
fn linked_multiedit_uses_primary_prepared_ratio() {
    let mut f = Fixture::new();
    f.set(layer("a", "scaleLinked"), true);
    f.select(&["a", "b"]);
    f.set(prop("a", "scale.x"), 200.0);
    assert_eq!(f.value("b", "scale.y"), 100.0.into());
}
#[test]
fn paired_coordinates_form_one_undoable_batch() {
    let mut f = Fixture::new();
    let r = f
        .model
        .set_many(
            Caller::native(),
            &[prop("a", "position.x"), prop("a", "position.y")],
            &[100.0.into(), 200.0.into()],
            2.0,
        )
        .unwrap();
    f.run(r);
    assert_eq!(f.value("a", "position.y"), 200.0.into());
    f.undo();
    assert_eq!(f.value("a", "position.x"), 10.0.into());
    assert_eq!(f.value("a", "position.y"), 20.0.into());
}
#[test]
fn effect_clipboard_renews_animated_parameter_and_enabled_key_ids() {
    let mut f = Fixture::new();
    act(&mut f, "a", "f1.amount", PropertyAction::Diamond);
    act(&mut f, "a", "f1.$enabled", PropertyAction::Diamond);
    f.model.copy_effects("a", "f1").unwrap();
    let (r, _) = f.model.paste_effects(Caller::native(), "b").unwrap();
    f.run(r);
    let src = f.model.layer("a").unwrap();
    let dst = f.model.layer("b").unwrap();
    let src = &src.get("fx").unwrap().as_array().unwrap()[0];
    let dst = dst.get("fx").unwrap().as_array().unwrap().last().unwrap();
    assert_ne!(
        src.get("on").unwrap().get("kf").unwrap(),
        dst.get("on").unwrap().get("kf").unwrap()
    );
    assert_ne!(
        src.get("p")
            .unwrap()
            .get("amount")
            .unwrap()
            .get("kf")
            .unwrap(),
        dst.get("p")
            .unwrap()
            .get("amount")
            .unwrap()
            .get("kf")
            .unwrap()
    );
}
#[test]
fn generated_origin_cannot_override_human_transform_intent() {
    let mut f = Fixture::new();
    f.set(prop("a", "position.x"), 70.0);
    let r = f
        .model
        .set(
            Caller::realm(5, 1, pm_kernel::Origin::Mcp),
            prop("a", "position.x"),
            90.0.into(),
            2.0,
        )
        .unwrap();
    let ack = f.handle.call_blocking("P", r).unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    f.sync();
    assert_eq!(f.value("a", "position.x"), 70.0.into());
}
#[test]
fn corner_toggle_seeds_all_corners_and_undo_restores() {
    let mut f = Fixture::new();
    let r = f
        .model
        .independent_corners(Caller::native(), "a", true, 2.0)
        .unwrap();
    f.run(r);
    let layer = f.model.layer("a").unwrap();
    let d = layer.get("d").unwrap();
    for key in ["radiusTL", "radiusTR", "radiusBL", "radiusBR"] {
        assert_eq!(d.get(key), Some(&10.0.into()));
    }
    assert_eq!(d.get("independentCorners"), Some(&true.into()));
    f.undo();
    assert!(
        f.model
            .layer("a")
            .unwrap()
            .get("d")
            .unwrap()
            .get("radiusTL")
            .is_none()
    );
}
#[test]
fn font_inspection_preserves_unknown_saved_axes_without_writing() {
    use pm_panel_inspector::content::*;
    let mut f = Fixture::new();
    let axis = FontAxis {
        tag: "TEST".into(),
        default: 0.5,
        min: None,
        max: None,
    };
    let r = f
        .model
        .set_axis(Caller::native(), "b", &axis, 0.7, 2.0, false)
        .unwrap();
    f.run(r);
    let revision = f.model.revision();
    let facts = FontFacts {
        axes: vec![FontAxis {
            tag: "wght".into(),
            default: 400.0,
            min: Some(100.0),
            max: Some(900.0),
        }],
        faces: vec![],
        readable: true,
    };
    let state = f.model.font_axes("b", &facts, 2.0).unwrap();
    assert_eq!(state.len(), 2);
    assert!(state.iter().any(|a| a.axis.tag == "TEST" && a.saved));
    assert_eq!(f.model.revision(), revision);
    assert!(
        f.model
            .layer("b")
            .unwrap()
            .get("d")
            .unwrap()
            .get("fontAxis.wght")
            .is_none()
    );
}
#[test]
fn axis_materialization_and_keyframe_are_one_undo() {
    use pm_panel_inspector::content::*;
    let mut f = Fixture::new();
    let axis = FontAxis {
        tag: "wght".into(),
        default: 400.0,
        min: Some(100.0),
        max: Some(900.0),
    };
    let r = f
        .model
        .set_axis(Caller::native(), "b", &axis, 500.0, 2.0, true)
        .unwrap();
    f.run(r);
    assert_eq!(f.value("b", "c.fontAxis.wght"), 500.0.into());
    assert!(
        f.model
            .binding_state(&prop("b", "c.fontAxis.wght"), 2.0)
            .unwrap()
            .key_at_playhead
    );
    f.undo();
    assert!(
        f.model
            .layer("b")
            .unwrap()
            .get("d")
            .unwrap()
            .get("fontAxis.wght")
            .is_none()
    );
}
#[test]
fn content_patch_routes_multiple_fields_atomically() {
    let mut f = Fixture::new();
    let mut patch = pm_model::Map::new();
    patch.insert("weight", 700.0.into());
    patch.insert("italic", true.into());
    let r = f
        .model
        .set_content_patch(Caller::native(), "b", &patch, 2.0)
        .unwrap();
    f.run(r);
    assert_eq!(f.value("b", "c.weight"), 700.0.into());
    assert_eq!(f.value("b", "c.italic"), true.into());
    f.undo();
    assert!(
        f.model
            .layer("b")
            .unwrap()
            .get("d")
            .unwrap()
            .get("italic")
            .is_none()
    );
}
#[test]
fn parent_candidates_exclude_self_and_descendants_but_include_groups() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let options = f.model.parent_candidates("a").unwrap();
    assert!(!options.contains(&"a".into()));
    assert!(options.contains(&"g".into()));
    let pick = f.model.begin_parent_pick("b", 2.0).unwrap();
    let r = f
        .model
        .finish_parent_pick(Caller::native(), pick, Some("a".into()), false)
        .unwrap()
        .unwrap();
    f.run(r);
    let options = f.model.parent_candidates("a").unwrap();
    assert!(!options.contains(&"b".into()));
}
#[test]
fn parent_pick_escape_produces_no_request_or_history() {
    let f = Fixture::new();
    let pick = f.model.begin_parent_pick("a", 2.0).unwrap();
    assert!(
        f.model
            .finish_parent_pick(Caller::native(), pick, Some("g".into()), true)
            .unwrap()
            .is_none()
    );
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn parent_pick_targets_are_captured_and_changes_are_undoable() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    let pick = f.model.begin_parent_pick("a", 2.0).unwrap();
    f.select(&["im"]);
    let r = f
        .model
        .finish_parent_pick(Caller::native(), pick, Some("im".into()), false)
        .unwrap()
        .unwrap();
    f.run(r);
    for id in ["a", "b"] {
        assert_eq!(f.model.layer(id).unwrap().get("parent"), Some(&"im".into()));
    }
    f.undo();
    assert!(
        f.model
            .layer("a")
            .unwrap()
            .get("parent")
            .is_none_or(Value::is_null)
    );
}
#[test]
fn matte_candidates_exclude_self_and_source_changes_are_undoable() {
    let mut f = Fixture::new();
    let options = f.model.matte_candidates("a").unwrap();
    assert!(!options.contains(&"a".into()));
    assert!(options.contains(&"b".into()));
    f.set(layer("a", "matteSource"), "b");
    assert_eq!(
        f.model.layer("a").unwrap().get("matteSource"),
        Some(&"b".into())
    );
    f.undo();
    assert!(
        f.model
            .layer("a")
            .unwrap()
            .get("matteSource")
            .is_none_or(Value::is_null)
    );
}
#[test]
fn graph_intents_use_ids_and_do_not_change_document() {
    let f = Fixture::new();
    let id = PropertyId::new("a", "scale.x");
    let intents = f.model.graph_intents(id.clone(), true, true).unwrap();
    assert_eq!(
        intents,
        vec![
            ServiceIntent::OpenGraph(id.clone()),
            ServiceIntent::Reveal(vec![id, PropertyId::new("a", "scale.y")])
        ]
    );
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn group_contents_selection_is_id_driven_even_when_locked() {
    let mut f = Fixture::new();
    let r = f
        .model
        .select_group_contents(Caller::native(), "g")
        .unwrap();
    f.run(r);
    assert_eq!(f.model.selected_layers(), ["child"]);
}
#[test]
fn effect_parameter_fallback_exposes_only_present_instance_fields() {
    let mut f = Fixture::new();
    let mut def = definition("blur");
    def.params.push(ParamDefinition {
        key: "newField".into(),
        kind: "number".into(),
        default: 1.0.into(),
        min: None,
        max: None,
        reference: false,
    });
    f.model.register_effect(def);
    let params = f.model.effect_parameters("a", "f1").unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].0, PropertyId::new("a", "f1.amount"));
}
fn add_video(f: &mut Fixture, speed: Value) {
    let mut content = pm_model::Map::new();
    content.insert("speed", speed);
    content.insert("trim", 2.0.into());
    let r = f
        .model
        .commands(
            Caller::native(),
            vec![
                pm_kernel::LayerCommand::Add(Box::new(pm_kernel::commands::layers::AddLayer {
                    id: "video".into(),
                    layer_type: "video".into(),
                    comp: None,
                    name: None,
                    from: Some(1.0),
                    duration: Some(4.0),
                    content,
                    properties: pm_model::Map::new(),
                    color: None,
                    index: None,
                    parent: None,
                    blend: None,
                    motion_blur: None,
                    visible: None,
                    shy: None,
                    collapsed: None,
                    time: 0.0,
                }))
                .into(),
            ],
        )
        .unwrap();
    f.run(r);
}
#[test]
fn remap_preserves_trim_and_speed_endpoints_and_undo() {
    use pm_panel_inspector::content::RemapMode;
    let mut f = Fixture::new();
    add_video(&mut f, 2.0.into());
    let before = f.model.layer("video").unwrap();
    let r = f
        .model
        .remap(Caller::native(), "video", RemapMode::Remap, 2.0)
        .unwrap();
    f.run(r);
    let layer = f.model.layer("video").unwrap();
    let keys = layer
        .get("d")
        .unwrap()
        .get("sourceTime")
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].get("v"), Some(&2.0.into()));
    assert!(
        (keys[1].get("v").unwrap().as_f64().unwrap() - (2.0 + (4.0 - 1.0 / 30.0) * 2.0)).abs()
            < 1e-10
    );
    f.undo();
    assert_eq!(f.model.layer("video").unwrap(), before);
}
#[test]
fn freeze_uses_current_source_time_and_hold_key() {
    use pm_panel_inspector::content::RemapMode;
    let mut f = Fixture::new();
    add_video(&mut f, 2.0.into());
    let r = f
        .model
        .remap(Caller::native(), "video", RemapMode::Freeze, 2.0)
        .unwrap();
    f.run(r);
    let layer = f.model.layer("video").unwrap();
    let keys = layer
        .get("d")
        .unwrap()
        .get("sourceTime")
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].get("v"), Some(&4.0.into()));
    assert_eq!(keys[0].get("hold"), Some(&true.into()));
}
#[test]
fn reverse_uses_reversed_source_endpoints() {
    use pm_panel_inspector::content::RemapMode;
    let mut f = Fixture::new();
    add_video(&mut f, 2.0.into());
    let r = f
        .model
        .remap(Caller::native(), "video", RemapMode::Reverse, 2.0)
        .unwrap();
    f.run(r);
    let layer = f.model.layer("video").unwrap();
    let keys = layer
        .get("d")
        .unwrap()
        .get("sourceTime")
        .unwrap()
        .get("kf")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(keys[1].get("v"), Some(&2.0.into()));
    assert!(keys[0].get("v").unwrap().as_f64().unwrap() > 9.0);
}
#[test]
fn remap_integrates_animated_speed_using_tag_midpoint_step() {
    use pm_panel_inspector::content::RemapMode;
    let mut f = Fixture::new();
    add_video(&mut f,parse(r#"{"v":2,"kf":[{"i":"s0","t":0,"v":2,"eo":[0,0],"ei":[1,1]},{"i":"s1","t":4,"v":2,"eo":[0,0],"ei":[1,1]}],"expr":null}"#).unwrap());
    let r = f
        .model
        .remap(Caller::native(), "video", RemapMode::Freeze, 2.0)
        .unwrap();
    f.run(r);
    assert!((f.value("video", "c.sourceTime").as_f64().unwrap() - 4.0).abs() < 1e-12);
}
#[test]
fn remap_refuses_wrong_layer_type_without_changes() {
    use pm_panel_inspector::content::RemapMode;
    let mut f = Fixture::new();
    assert!(
        f.model
            .remap(Caller::native(), "a", RemapMode::Remap, 0.0)
            .is_err()
    );
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn import_source_requires_registered_matching_kind_and_preserves_selection() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    assert!(
        f.model
            .imported_source(Caller::native(), "im", "video", "missing", 0.0)
            .is_err()
    );
    assert!(
        f.model
            .imported_source(Caller::native(), "im", "image", "missing", 0.0)
            .is_err()
    );
    assert_eq!(f.model.selected_layers(), ["a"]);
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn global_effect_paste_targets_all_raw_selected_layers_once() {
    let mut f = Fixture::new();
    f.model.copy_effects("a", "f1").unwrap();
    f.select(&["a", "b"]);
    let r = f.model.paste_selected_effects(Caller::native()).unwrap();
    f.run(r);
    assert_eq!(f.model.effect_ids("a").unwrap().len(), 4);
    assert_eq!(f.model.effect_ids("b").unwrap().len(), 2);
    f.undo();
    assert_eq!(f.model.effect_ids("a").unwrap().len(), 3);
    assert_eq!(f.model.effect_ids("b").unwrap().len(), 1);
}
#[test]
fn folded_foreign_edit_survives_gesture_cancel() {
    let mut f = Fixture::new();
    let (mut s, r) = f
        .model
        .begin_edit(Caller::native(), &[prop("a", "opacity")], 2.0)
        .unwrap();
    f.run(r);
    let r = f
        .model
        .edit_step(Caller::native(), &mut s, &[50.0.into()])
        .unwrap();
    f.run(r);
    let r = f
        .model
        .set(
            Caller::realm(3, 1, pm_kernel::Origin::Agent),
            prop("b", "opacity"),
            30.0.into(),
            2.0,
        )
        .unwrap();
    f.run(r);
    f.run(f.model.finish_edit(Caller::native(), true));
    assert_eq!(f.value("a", "opacity"), 100.0.into());
    assert_eq!(f.value("b", "opacity"), 30.0.into());
    f.undo();
    assert_eq!(f.value("b", "opacity"), 100.0.into());
}
#[test]
fn gesture_rejects_another_writer_before_dispatch() {
    let mut f = Fixture::new();
    let (mut s, r) = f
        .model
        .begin_edit(Caller::native(), &[prop("a", "opacity")], 2.0)
        .unwrap();
    f.run(r);
    assert!(
        f.model
            .edit_step(
                Caller::realm(9, 1, pm_kernel::Origin::Mcp),
                &mut s,
                &[1.0.into()]
            )
            .is_err()
    );
    f.run(f.model.finish_edit(Caller::native(), true));
    assert_eq!(f.value("a", "opacity"), 100.0.into());
}
#[test]
fn gesture_rejects_replaced_document_even_with_same_layer_ids() {
    let mut f = Fixture::new();
    let (mut s, r) = f
        .model
        .begin_edit(Caller::native(), &[prop("a", "opacity")], 2.0)
        .unwrap();
    f.run(r);
    f.run(f.model.finish_edit(Caller::native(), true));
    let project = f
        .handle
        .published("P")
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value();
    assert!(f.handle.close("P").wait().unwrap());
    f.handle
        .open("P", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    f.sync();
    assert!(
        f.model
            .edit_step(Caller::native(), &mut s, &[1.0.into()])
            .is_err()
    );
}
#[test]
fn semantic_catalog_exposes_composition_without_eager_writes() {
    let f = Fixture::new();
    let fields = f.model.fields().unwrap();
    assert!(
        fields
            .iter()
            .any(|d| d.binding == Binding::Composition("width".into()))
    );
    assert!(!f.handle.published("P").unwrap().history.can_undo);
}
#[test]
fn semantic_catalog_tracks_independent_corner_availability() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    assert!(
        f.model
            .fields()
            .unwrap()
            .iter()
            .any(|d| d.binding == content("a", "radius"))
    );
    let r = f
        .model
        .independent_corners(Caller::native(), "a", true, 2.0)
        .unwrap();
    f.run(r);
    let fields = f.model.fields().unwrap();
    assert!(!fields.iter().any(|d| d.binding == content("a", "radius")));
    assert!(fields.iter().any(|d| d.binding == content("a", "radiusTL")));
}
#[test]
fn semantic_catalog_does_not_offer_video_audio_without_soundtrack() {
    let mut f = Fixture::new();
    add_video(&mut f, 1.0.into());
    f.select(&["video"]);
    assert!(
        !f.model
            .fields()
            .unwrap()
            .iter()
            .any(|d| d.binding == content("video", "audioGain"))
    );
    f.set(content("video", "embeddedAudio"), true);
    assert!(
        f.model
            .fields()
            .unwrap()
            .iter()
            .any(|d| d.binding == content("video", "audioGain"))
    );
}

#[test]
fn imported_matching_source_applies_to_captured_layer_and_undo() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let r = f
        .model
        .imported_source(Caller::native(), "im", "image", "img", 2.0)
        .unwrap();
    f.run(r);
    assert_eq!(
        f.model.layer("im").unwrap().get("d").unwrap().get("asset"),
        Some(&"img".into())
    );
    assert_eq!(f.model.selected_layers(), ["a"]);
    f.undo();
    assert!(
        f.model
            .layer("im")
            .unwrap()
            .get("d")
            .unwrap()
            .get("asset")
            .is_none()
    );
}
