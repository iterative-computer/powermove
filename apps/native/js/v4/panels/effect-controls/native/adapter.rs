use pm_kernel::{
    Caller, HistoryCall, KernelActor, KernelConfig, KernelHandle, OpenSource, Reply, Request,
    SelectionCall,
};
use pm_model::{Map, Value, json::parse};
use pm_panel_inspector::{
    EffectClipboard, EffectDefinition, Inspector, ParamDefinition,
    v4_effects::{EffectsPanel, Presentation},
};
struct Fixture {
    actor: Option<KernelActor>,
    handle: KernelHandle,
    model: Inspector,
    panel: EffectsPanel,
    initial_revision: u64,
}
impl Fixture {
    fn new(project: Value, range: std::ops::Range<usize>) -> Self {
        let actor = KernelActor::spawn(KernelConfig::default());
        let handle = actor.handle();
        handle
            .open("P", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        let model = Inspector::new(handle.published("P").unwrap(), EffectClipboard::default());
        let initial_revision = model.revision();
        Self {
            actor: Some(actor),
            handle,
            model,
            panel: EffectsPanel::new(Presentation::default(), range),
            initial_revision,
        }
    }
    fn sync(&mut self) {
        self.model.sync(self.handle.published("P").unwrap());
    }
    fn select(&mut self, id: &str) {
        self.handle
            .call(
                "P",
                Request::new(
                    Caller::native(),
                    SelectionCall::Layers {
                        ids: vec![id.into()],
                        add: false,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        self.sync();
    }
    fn args(&self, intent: &str) -> Value {
        let mut args = Map::new();
        let p = self.handle.published("P").unwrap();
        args.insert("documentId", (p.snapshot.doc_id as f64).into());
        args.insert("revision", (p.revision() as f64).into());
        args.insert("layer", self.model.primary().unwrap_or_default().into());
        args.insert("intent", parse(intent).unwrap());
        Value::Object(args)
    }
    fn step(&mut self, intent: &str) {
        let args = self.args(intent);
        let mut plan = self
            .panel
            .plan(&mut self.model, Caller::native(), &args, 0.0)
            .unwrap();
        if let Some(request) = plan.request.take() {
            let ack = self
                .model
                .submit(&self.handle, request)
                .wait()
                .unwrap()
                .unwrap();
            self.sync();
            self.panel.acknowledge(&mut self.model, plan, &ack).unwrap();
        }
    }
    fn project(&mut self) -> Value {
        self.panel.project(&mut self.model, 0.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.actor.take().unwrap().shutdown();
    }
}
fn project() -> Value {
    parse(r#"{"id":"P","compId":"C","w":640,"h":360,"fps":30,"dur":10,"layers":[{"id":"a","type":"shape","name":"A","p":{},"d":{},"fx":[{"id":"f1","type":"glow","on":true,"open":true,"p":{"radius":{"v":7,"kf":[],"expr":null}}},{"id":"f2","type":"glow","on":true,"open":true,"p":{"radius":{"v":8,"kf":[],"expr":null}}},{"id":"missing","type":"not-installed","on":true,"p":{"radius":{"v":9,"opaque":true}}}]},{"id":"locked","type":"shape","lock":true,"p":{},"d":{},"fx":[]},{"id":"audio","type":"audio","p":{},"d":{},"fx":[]}]}"#).unwrap()
}
fn definition(params: Vec<ParamDefinition>) -> EffectDefinition {
    EffectDefinition {
        effect_type: "glow".into(),
        version: 1,
        label: "Glow".into(),
        source_processing: false,
        backdrop: false,
        params,
    }
}
fn param(key: String) -> ParamDefinition {
    ParamDefinition {
        key,
        kind: "number".into(),
        default: 0.into(),
        min: None,
        max: None,
        reference: false,
    }
}
fn rows(value: &Value) -> &[Value] {
    value.get("rows").and_then(Value::as_array).unwrap()
}
#[test]
fn bounded_projection_and_target_revision_document_fences() {
    let mut f = Fixture::new(project(), 0..1);
    assert_eq!(
        f.project().get("target").and_then(Value::as_str),
        Some("NoSelection")
    );
    f.select("a");
    assert_eq!(f.project().get("count").and_then(Value::as_f64), Some(6.0));
    assert_eq!(rows(&f.project()).len(), 1);
    assert!(f.panel.project(&mut f.model, f64::NAN).is_err());
    for key in ["layer", "revision", "documentId"] {
        let mut args = f.args(r#"{"type":"clear"}"#);
        args.as_object_mut().unwrap().insert(
            key,
            if key == "layer" {
                "other".into()
            } else {
                99.into()
            },
        );
        assert!(
            f.panel
                .plan(&mut f.model, Caller::native(), &args, 0.0)
                .is_err()
        );
    }
}
#[test]
fn missing_definitions_remain_raw_and_readonly_without_document_mutation() {
    let mut f = Fixture::new(project(), 0..32);
    f.select("a");
    let p = f.project();
    let raw = rows(&p)
        .iter()
        .find(|r| {
            r.get("parameter")
                .and_then(|p| p.get("effect"))
                .and_then(Value::as_str)
                == Some("missing")
        })
        .unwrap();
    let parameter = raw.get("parameter").unwrap();
    assert_eq!(
        parameter.get("readonly").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        parameter
            .get("value")
            .unwrap()
            .get("opaque")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(f.model.revision(), f.initial_revision);
}
#[test]
fn cached_structure_tracks_registry_changes_at_same_revision() {
    let mut f = Fixture::new(project(), 0..32);
    f.select("a");
    assert_eq!(
        rows(&f.project())[0]
            .get("missing")
            .and_then(Value::as_bool),
        Some(true)
    );
    f.model
        .register_effect(definition(vec![param("radius".into())]));
    assert_eq!(
        rows(&f.project())[0]
            .get("missing")
            .and_then(Value::as_bool),
        Some(false)
    );
    f.model.register_effect(definition(vec![]));
    assert_eq!(f.project().get("count").and_then(Value::as_f64), Some(4.0));
}
#[test]
fn scroll_anchors_follow_ids_through_reorder() {
    let mut f = Fixture::new(project(), 2..3);
    f.select("a");
    assert_eq!(
        rows(&f.project())[0].get("id").and_then(Value::as_str),
        Some("f2")
    );
    f.step(r#"{"type":"drop","id":"f1","slot":3}"#);
    let p = f.project();
    assert_eq!(rows(&p)[0].get("id").and_then(Value::as_str), Some("f2"));
    assert_eq!(
        p.get("range").and_then(Value::as_array).unwrap()[0].as_f64(),
        Some(0.0)
    );
}
#[test]
fn locked_and_unsupported_targets_reject_writes() {
    let mut f = Fixture::new(project(), 0..32);
    f.model.register_effect(definition(vec![]));
    for id in ["locked", "audio"] {
        f.select(id);
        let args = f.args(r#"{"type":"add","definition":"glow"}"#);
        assert!(
            f.panel
                .plan(&mut f.model, Caller::native(), &args, 0.0)
                .is_err()
        );
    }
    assert_eq!(f.model.revision(), f.initial_revision);
}
#[test]
fn proposed_selection_is_not_installed_on_kernel_refusal() {
    let mut f = Fixture::new(project(), 0..32);
    f.select("a");
    f.step(r#"{"type":"select","id":"f1","meta":false,"ctrl":false,"shift":false}"#);
    let before = f.model.effects.clone();
    let args = f.args(r#"{"type":"remove","id":"f1"}"#);
    let mut plan = f
        .panel
        .plan(&mut f.model, Caller::native(), &args, 0.0)
        .unwrap();
    assert_eq!(f.model.effects, before);
    let request = plan.request.take().unwrap();
    f.step(r#"{"type":"toggle","id":"f2"}"#);
    let ack = f.model.submit(&f.handle, request).wait().unwrap().unwrap();
    f.sync();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    assert!(f.panel.acknowledge(&mut f.model, plan, &ack).is_err());
    assert_eq!(f.model.effects, before);
}
#[test]
fn caller_identity_and_exact_undo_label_are_retained() {
    let mut f = Fixture::new(project(), 0..32);
    f.select("a");
    let args = f.args(r#"{"type":"toggle","id":"f1"}"#);
    let caller = Caller::realm(42, 7, pm_kernel::Origin::Agent);
    let plan = f.panel.plan(&mut f.model, caller, &args, 0.0).unwrap();
    let request = plan.request.unwrap();
    assert_eq!(request.caller, caller);
    if let pm_kernel::Call::Transact {
        label,
        expected_revision,
        ..
    } = request.call
    {
        assert_eq!(label.as_deref(), Some("Disable Effect"));
        assert_eq!(expected_revision, Some(f.initial_revision));
    } else {
        panic!("typed transaction required");
    }
}
#[test]
fn sixty_thousand_parameters_evaluate_only_requested_window() {
    let mut p = project();
    let layer = &mut p
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap()[0];
    let fx = &mut layer
        .as_object_mut()
        .unwrap()
        .get_mut("fx")
        .unwrap()
        .as_array_mut()
        .unwrap()[0];
    let mut values = Map::new();
    let params = (0..60_000)
        .map(|i| {
            let key = format!("p{i}");
            let mut value = Map::new();
            value.insert("v", (i as f64).into());
            value.insert("kf", Value::Array(vec![]));
            value.insert(
                "expr",
                if i == 59_999 {
                    "throw new Error('hidden')".into()
                } else {
                    Value::Null
                },
            );
            values.insert(key.clone(), Value::Object(value));
            param(key)
        })
        .collect();
    fx.as_object_mut()
        .unwrap()
        .insert("p", Value::Object(values));
    let mut f = Fixture::new(p, 0..1);
    f.model.register_effect(definition(params));
    f.select("a");
    let p = f.project();
    assert_eq!(p.get("count").and_then(Value::as_f64), Some(60_004.0));
    f.step(r#"{"type":"range","range":[30000,30003]}"#);
    let p = f.project();
    assert_eq!(rows(&p).len(), 3);
    assert_eq!(
        rows(&p)[0]
            .get("parameter")
            .and_then(|p| p.get("value"))
            .and_then(Value::as_f64),
        Some(29_999.0)
    );
}
#[test]
fn empty_clipboard_and_invalid_insertions_fail_without_editing() {
    let mut f = Fixture::new(project(), 0..32);
    f.select("a");
    for intent in [
        r#"{"type":"paste"}"#,
        r#"{"type":"drop","id":"f1","slot":-1}"#,
        r#"{"type":"range","range":[1,0]}"#,
        r#"{"type":"parameter","id":"{\"layer\":\"other\",\"path\":\"f1.radius\"}","action":"diamond"}"#,
    ] {
        let args = f.args(intent);
        assert!(
            f.panel
                .plan(&mut f.model, Caller::native(), &args, 0.0)
                .is_err()
        );
    }
    assert_eq!(f.model.revision(), f.initial_revision);
    // The real kernel remains the history owner; a refused edit adds no Undo.
    let ack = f
        .handle
        .call("P", Request::new(Caller::native(), HistoryCall::Undo))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(ack.revision, f.initial_revision);
}

#[test]
fn replaced_document_rejects_old_intent_and_rebuilds_same_revision_rows() {
    let mut f = Fixture::new(project(), 0..32);
    f.select("a");
    let old = f.args(r#"{"type":"toggle","id":"f1"}"#);
    let before = f.project();
    assert!(f.handle.close("P").wait().unwrap());
    let mut replacement = project();
    let layer = &mut replacement
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap()[0];
    let fx = &mut layer
        .as_object_mut()
        .unwrap()
        .get_mut("fx")
        .unwrap()
        .as_array_mut()
        .unwrap()[0];
    fx.as_object_mut()
        .unwrap()
        .insert("p", parse(r#"{"other":{"v":23}}"#).unwrap());
    f.handle
        .open("P", OpenSource::Untitled(replacement))
        .wait()
        .unwrap()
        .unwrap();
    f.sync();
    f.select("a");
    let after = f.project();
    assert_ne!(after.get("documentId"), before.get("documentId"));
    assert_eq!(
        rows(&after)[1]
            .get("parameter")
            .and_then(|p| p.get("label"))
            .and_then(Value::as_str),
        Some("other")
    );
    assert!(
        f.panel
            .plan(&mut f.model, Caller::native(), &old, 0.0)
            .is_err()
    );
}
#[test]
fn option_diamond_on_static_parameter_is_a_noop_and_does_not_add_animation() {
    let mut f = Fixture::new(project(), 0..32);
    f.select("a");
    f.model
        .register_effect(definition(vec![param("radius".into())]));
    f.step(r#"{"type":"parameter","id":"{\"layer\":\"a\",\"path\":\"f1.radius\"}","action":"stopwatch"}"#);
    assert_eq!(f.model.revision(), f.initial_revision);
}
