use pm_kernel::{
    Call, Caller, HistoryCall, KernelActor, KernelConfig, KernelHandle, OpenSource, Reply, Request,
    SelectionCall,
};
use pm_model::{Value, json::parse};
use pm_panel_inspector::{
    Binding, EffectClipboard, Inspector, InspectorError, PropertyId, v4_properties::*,
};

struct Fixture {
    actor: Option<KernelActor>,
    kernel: KernelHandle,
    model: Inspector,
    adapter: V4Adapter,
}
impl Fixture {
    fn new() -> Self {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = parse(r##"{"id":"P","compId":"C","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[
          {"id":"a","name":"A","type":"shape","on":true,"lock":false,"from":0,"dur":10,"p":{"opacity":{"v":100,"kf":[],"expr":null},"scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":50,"kf":[],"expr":null}},"d":{"color":"#ff0000","w":100,"h":100},"fx":[],"masks":[]},
          {"id":"b","name":"B","type":"text","on":true,"lock":false,"from":0,"dur":10,"p":{"opacity":{"v":50,"kf":[],"expr":null},"scale.x":{"v":100,"kf":[],"expr":null},"scale.y":{"v":100,"kf":[],"expr":null}},"d":{"color":"#00ff00","size":64,"text":"authored"},"fx":[],"masks":[]},
          {"id":"g","name":"G","type":"group","lock":true,"on":true,"from":0,"dur":10,"p":{},"d":{},"fx":[],"masks":[]},
          {"id":"child","name":"Child","type":"shape","group":"g","on":true,"from":0,"dur":10,"p":{"opacity":{"v":25,"kf":[],"expr":null}},"d":{"color":"#0000ff","w":100},"fx":[],"masks":[]}
        ]}"##).unwrap();
        kernel
            .open("P", OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        let model = Inspector::new(kernel.published("P").unwrap(), EffectClipboard::default());
        Self {
            actor: Some(actor),
            kernel,
            model,
            adapter: V4Adapter::default(),
        }
    }
    fn run(&mut self, request: Request) {
        let ack = self
            .model
            .submit(&self.kernel, request)
            .wait()
            .unwrap()
            .unwrap();
        assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
        self.model.sync(self.kernel.published("P").unwrap());
    }
    fn select(&mut self, ids: &[&str]) {
        self.run(Request::new(
            Caller::native(),
            SelectionCall::Layers {
                ids: ids.iter().map(|id| (*id).into()).collect(),
                add: false,
            },
        ));
    }
    fn handle(&mut self, layer: &str, path: &str) -> String {
        self.adapter
            .register(
                &self.model,
                vec![Binding::Property(PropertyId::new(layer, path))],
            )
            .unwrap()
    }
    fn action(&mut self, handle: &str, action: V4Action, time: f64) {
        match self
            .adapter
            .action(&mut self.model, Caller::native(), handle, action, time)
            .unwrap()
        {
            V4Plan::Request(request) => self.run(request),
            _ => panic!("Expected actor request"),
        }
    }
    fn value(&mut self, layer: &str, path: &str) -> Value {
        self.model
            .binding_state(&Binding::Property(PropertyId::new(layer, path)), 1.)
            .unwrap()
            .value
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.actor.take().unwrap().shutdown();
    }
}

#[test]
fn v4_absolute_multi_edit_skips_locked_ancestors_and_undoes_once() {
    let mut f = Fixture::new();
    f.select(&["a", "b", "child"]);
    let handle = f.handle("a", "opacity");
    let before = f.adapter.read(&mut f.model, &handle, 1.).unwrap();
    assert!(before[0].mixed);
    let request = f
        .adapter
        .set(&mut f.model, Caller::native(), &handle, &[80.into()], 1.)
        .unwrap();
    f.run(request);
    assert_eq!(f.value("a", "opacity"), 80.into());
    assert_eq!(f.value("b", "opacity"), 80.into());
    assert_eq!(f.value("child", "opacity"), 25.into());
    f.run(Request::new(
        Caller::native(),
        Call::History(HistoryCall::Undo),
    ));
    assert_eq!(f.value("a", "opacity"), 100.into());
    assert_eq!(f.value("b", "opacity"), 50.into());
}
#[test]
fn v4_diamond_only_toggles_playhead_key_remove_keeps_current_value() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let handle = f.handle("a", "opacity");
    f.action(&handle, V4Action::Diamond, 1.);
    f.action(&handle, V4Action::Diamond, 2.);
    f.action(&handle, V4Action::Diamond, 2.);
    let at_one = f.adapter.read(&mut f.model, &handle, 1.).unwrap();
    assert!(at_one[0].animated && at_one[0].key_at_playhead);
    assert!(!f.adapter.read(&mut f.model, &handle, 2.).unwrap()[0].key_at_playhead);
    f.action(&handle, V4Action::RemoveAnimation, 1.);
    let after = f.adapter.read(&mut f.model, &handle, 1.).unwrap();
    assert!(!after[0].animated);
    assert_eq!(after[0].value, 100.into());
}
#[test]
fn v4_remove_animation_on_static_property_never_creates_animation() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let handle = f.handle("a", "opacity");
    assert!(matches!(
        f.adapter.action(
            &mut f.model,
            Caller::native(),
            &handle,
            V4Action::RemoveAnimation,
            1.
        ),
        Err(InspectorError::EmptyEdit)
    ));
    assert!(!f.adapter.read(&mut f.model, &handle, 1.).unwrap()[0].animated);
}
#[test]
fn v4_stale_selection_and_disposed_handles_refuse_new_work() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let old = f.handle("a", "opacity");
    f.select(&["b"]);
    assert!(f.adapter.read(&mut f.model, &old, 1.).is_err());
    assert!(
        f.adapter
            .set(&mut f.model, Caller::native(), &old, &[1.into()], 1.)
            .is_err()
    );
    let fresh = f.handle("b", "opacity");
    assert_ne!(old, fresh);
    f.adapter.dispose(&fresh);
    assert!(f.adapter.read(&mut f.model, &fresh, 1.).is_err());
    let latest = f.handle("b", "opacity");
    f.adapter.retire();
    assert!(f.adapter.read(&mut f.model, &latest, 1.).is_err());
}
#[test]
fn v4_live_edit_captures_targets_and_time_across_selection_change_and_cancel() {
    let mut f = Fixture::new();
    f.select(&["a", "b"]);
    let handle = f.handle("a", "opacity");
    let (mut edit, begin) = f
        .adapter
        .begin(&mut f.model, Caller::native(), &handle, 1.)
        .unwrap();
    f.run(begin);
    f.select(&["child"]);
    let step = f
        .model
        .edit_step(Caller::native(), &mut edit, &[90.into()])
        .unwrap();
    f.run(step);
    assert_eq!(f.value("a", "opacity"), 90.into());
    assert_eq!(f.value("b", "opacity"), 40.into());
    assert_eq!(f.value("child", "opacity"), 25.into());
    f.run(f.model.finish_edit(Caller::native(), true));
    assert_eq!(f.value("a", "opacity"), 100.into());
    assert_eq!(f.value("b", "opacity"), 50.into());
}
#[test]
fn v4_actor_request_preserves_real_agent_caller_and_finite_validation() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let handle = f.handle("a", "opacity");
    let caller = Caller::realm(9, 2, pm_kernel::Origin::Mcp);
    let request = f
        .adapter
        .set(&mut f.model, caller, &handle, &[80.into()], 1.)
        .unwrap();
    assert_eq!(request.caller, caller);
    assert!(f.adapter.read(&mut f.model, &handle, f64::NAN).is_err());
    assert!(f.adapter.register(&f.model, vec![]).is_err());
}
#[test]
fn v4_graph_action_is_a_service_intent_and_does_not_mutate_the_document() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let handle = f.handle("a", "opacity");
    let revision = f.model.revision();
    match f
        .adapter
        .action(
            &mut f.model,
            Caller::native(),
            &handle,
            V4Action::ShowGraph,
            1.,
        )
        .unwrap()
    {
        V4Plan::Service(intents) => assert_eq!(intents.len(), 2),
        _ => panic!("Expected service intents"),
    }
    assert_eq!(f.model.revision(), revision);
}
#[test]
fn v4_paired_axes_use_one_model_transaction_and_keep_authored_values() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let handle = f
        .adapter
        .register(
            &f.model,
            vec![
                Binding::Property(PropertyId::new("a", "scale.x")),
                Binding::Property(PropertyId::new("a", "scale.y")),
            ],
        )
        .unwrap();
    let request = f
        .adapter
        .set(
            &mut f.model,
            Caller::native(),
            &handle,
            &[120.into(), 75.into()],
            1.,
        )
        .unwrap();
    f.run(request);
    assert_eq!(f.value("a", "scale.x"), 120.into());
    assert_eq!(f.value("a", "scale.y"), 75.into());
    f.run(Request::new(
        Caller::native(),
        Call::History(HistoryCall::Undo),
    ));
    assert_eq!(f.value("a", "scale.x"), 100.into());
    assert_eq!(f.value("a", "scale.y"), 50.into());
}

#[test]
fn v4_remove_animation_finds_animated_secondary_when_primary_is_static() {
    let mut f = Fixture::new();
    f.select(&["b"]);
    let secondary = f.handle("b", "opacity");
    f.action(&secondary, V4Action::Diamond, 1.);
    f.select(&["a", "b"]);
    let primary = f.handle("a", "opacity");
    f.action(&primary, V4Action::RemoveAnimation, 1.);
    assert!(!f.adapter.read(&mut f.model, &primary, 1.).unwrap()[0].animated);
    f.select(&["b"]);
    let result = f.handle("b", "opacity");
    assert!(!f.adapter.read(&mut f.model, &result, 1.).unwrap()[0].animated);
}
#[test]
fn v4_link_action_refuses_non_scale_handles() {
    let mut f = Fixture::new();
    f.select(&["a"]);
    let opacity = f.handle("a", "opacity");
    assert!(
        f.adapter
            .action(
                &mut f.model,
                Caller::native(),
                &opacity,
                V4Action::LinkScale,
                1.
            )
            .is_err()
    );
    let scale = f.handle("a", "scale.x");
    f.action(&scale, V4Action::LinkScale, 1.);
}
