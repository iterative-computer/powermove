//! The application catalog with nothing registered by the test: panels read
//! what `Kernel::new` seeds, and effects reach a rendered frame.
use super::*;
use pm_kernel::{Call, WriteBody};

fn setup(fx: Value) -> KernelActor {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = json!({
        "id":"synthetic","name":"Registry","compId":"root","w":8,"h":8,"fps":30,"dur":2,
        "colorDepth":16,"bg":"#000000","backgroundFill":{"type":"none","angle":0,"stops":[]},
        "assets":{},"comps":{},
        "layers":[{"id":"a","type":"solid","name":"Plate","from":0,"dur":2,"on":true,
            "parent":null,"group":null,
            "p":{"position.x":{"v":4,"kf":[]},"position.y":{"v":4,"kf":[]},
                "anchor.x":{"v":4,"kf":[]},"anchor.y":{"v":4,"kf":[]},
                "scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},
                "rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},
            "d":{"w":8,"h":8,"color":"#FF0000"},"fx":fx}]
    });
    actor
        .handle()
        .open(
            "synthetic",
            OpenSource::Untitled(data::model(&project).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    select_layer(&actor.handle(), "a");
    actor
}
fn pixel(kernel: &pm_kernel::KernelHandle, renderer: &mut pm_render::Renderer) -> [f32; 4] {
    pixel_with_extensions(kernel, renderer, &[])
}
fn pixel_with_extensions(
    kernel: &pm_kernel::KernelHandle,
    renderer: &mut pm_render::Renderer,
    definitions: &[pm_protocol::definitions::DefinitionSnapshot],
) -> [f32; 4] {
    let p = kernel.published("synthetic").unwrap();
    let _turn = install_main_effects(renderer, &p.snapshot.document);
    if !definitions.is_empty() {
        assert!(
            renderer
                .set_document_effects(&p.snapshot.document, definitions)
                .unwrap()
                .is_empty()
        );
    }
    let mut request = pm_render::RenderRequest::sized(
        pm_protocol::common::Size::new(8, 8),
        pm_protocol::color::Precision::Float16,
    );
    request.export = true;
    request.output_kind = pm_render::OutputKind::Final;
    let frame = pm_export::render::render_frame(
        renderer,
        &p.snapshot.document,
        "root",
        pm_protocol::common::Ticks::new(0).unwrap(),
        &request,
        &Default::default(),
        &pm_export::render::Unrestricted,
        1,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    frame.rgba[(4 * 8 + 4) * 4..][..4].try_into().unwrap()
}
fn effects(kernel: &pm_kernel::KernelHandle) -> Vec<Value> {
    let p = kernel.published("synthetic").unwrap();
    let root = p.snapshot.root().unwrap();
    let layer = pm_kernel::model::layer_at(root, &pm_kernel::model::find_layer(root, "a").unwrap())
        .unwrap()
        .to_value();
    data::wire(&layer)["fx"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}
fn texts(surface: &Surface) -> Vec<String> {
    surface.nodes().values().map(|n| n.text.clone()).collect()
}

#[test]
fn transition_properties_note_tracks_effective_handles_and_undo() {
    use pm_kernel::commands::timing::{TimeEdge, TimingCommand};
    let actor = setup(json!([]));
    let kernel = actor.handle();
    let project = pm_model::canonical::realize_project(
        kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
    );
    let mut project = data::wire(&project);
    project["dur"] = json!(3);
    project["assets"] = json!({"source-a":{"id":"source-a","kind":"video","dur":1.8},"source-b":{"id":"source-b","kind":"video","dur":2}});
    project["layers"] = json!([
        {"id":"a","type":"video","name":"A","from":0,"dur":1.5,"track":0,"on":true,"p":{},"fx":[],"d":{"asset":"source-a","trim":0,"speed":1}},
        {"id":"b","type":"video","name":"B","from":1.5,"dur":1.5,"track":0,"on":true,"p":{},"fx":[],"d":{"asset":"source-b","trim":0.3,"speed":1}}
    ]);
    assert!(kernel.close("synthetic").wait().unwrap());
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(data::model(&project).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    select_layer(&kernel, "b");
    let run = |commands: Vec<pm_kernel::Command>| {
        let ack = kernel
            .call(
                "synthetic",
                pm_kernel::Request::new(
                    pm_kernel::Caller::native(),
                    Call::Transact {
                        label: Some("Edit Clip".into()),
                        expected_revision: None,
                        body: WriteBody::Commands(commands),
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert!(
            !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
            "{:?}",
            ack.reply
        );
    };
    run(vec![
        pm_kernel::EffectCommand::SetPairedTransition {
            left: "a".into(),
            right: "b".into(),
            offset: None,
            transition: pm_kernel::commands::transition_edges::builtin_spec("crossfade")
                .map(Box::new),
        }
        .into(),
    ]);
    let check = |shortened: bool, zero: bool| {
        let mut inspector = pm_panel_inspector::Inspector::new(
            kernel.published("synthetic").unwrap(),
            Default::default(),
        );
        let snapshot = crate::properties::Properties::default()
            .snapshot(&mut inspector, 0., false)
            .unwrap();
        let section = snapshot["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "transition-in")
            .unwrap();
        assert_eq!(
            section["note"]
                .as_str()
                .is_some_and(|s| s.starts_with("Shortened to fit media")),
            shortened
        );
        if zero {
            assert!(section["note"].as_str().unwrap().contains("plain cut"));
        }
        let duration = section["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "transition.in.duration")
            .unwrap();
        assert_eq!(
            duration["control"]["axes"][0]["value"], 0.5,
            "Properties edits the retained intent"
        );
    };
    check(false, false);
    run(vec![
        TimingCommand::Roll {
            layer_id: "a".into(),
            edge: TimeEdge::Out,
            time: 1.7,
        }
        .into(),
    ]);
    check(true, false);
    kernel
        .call(
            "synthetic",
            pm_kernel::Request::new(pm_kernel::Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    check(false, false);
    run(vec![
        TimingCommand::Roll {
            layer_id: "a".into(),
            edge: TimeEdge::Out,
            time: 1.8,
        }
        .into(),
        pm_kernel::PropertyCommand::Set(pm_kernel::commands::properties::SetProperty::new(
            "b", "c.trim", 0., 1.8,
        ))
        .into(),
    ]);
    check(true, true);
}

#[test]
fn effects_panel_lists_seeded_builtins_in_registration_order() {
    let actor = setup(json!([]));
    let kernel = actor.handle();
    let state = kernel.state();
    let seeded = state.registries.get("effects").unwrap();
    assert_eq!(
        seeded.len(),
        pm_fx_defs::catalog::main_effects().effects.len()
    );
    assert_eq!(seeded[0].id, "blur");
    let mut panels = Builtins::new(kernel, "synthetic", || {}).unwrap();
    finish(panels.mount("fxbrowser").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let shown = texts(&publication.surface);
    for label in [
        "Blur & Sharpen",
        "Gaussian Blur",
        "Channel",
        "Transitions",
        "Crossfade",
        "Wipe",
        "Slide",
        "Zoom",
    ] {
        assert!(shown.iter().any(|t| t == label), "{label}: {shown:?}");
    }
    assert!(!shown.iter().any(|t| t == "No Effects"), "{shown:?}");
}

#[test]
fn transitions_panel_add_button_preserves_the_v1_edge_model() {
    let actor = setup(json!([]));
    let kernel = actor.handle();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("fxbrowser").unwrap());
    for label in ["Crossfade, transition, Transitions", "Add Crossfade"] {
        let p = panels.take_publications().pop().unwrap();
        let node = p
            .surface
            .nodes()
            .values()
            .find(|n| n.props.get("label").and_then(Value::as_str) == Some(label))
            .unwrap_or_else(|| panic!("no {label} button"));
        finish(
            panels
                .event(
                    "fxbrowser",
                    p.generation,
                    p.surface.revision(),
                    node.id,
                    "click",
                    EventDetail::default(),
                )
                .unwrap(),
        );
    }
    let transition = || {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap();
        pm_kernel::model::layer_at(root, &pm_kernel::model::find_layer(root, "a").unwrap())
            .unwrap()
            .get("transitionIn")
            .map(pm_model::Node::to_value)
            .filter(|v| !matches!(v, pm_model::Value::Null))
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while transition().is_none() {
        assert!(
            Instant::now() < deadline,
            "transition add did not settle: {:?}",
            panels.take_errors()
        );
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
        thread::sleep(Duration::from_millis(1));
    }
    let edge = transition().unwrap();
    assert_eq!(
        edge.get("type").and_then(pm_model::Value::as_str),
        Some("crossfade")
    );
    assert_eq!(edge.get("dur").and_then(pm_model::Value::as_f64), Some(0.5));
    assert!(edge.get("paired").is_none());
    assert!(
        effects(&kernel).is_empty(),
        "a transition is not an effect node"
    );
    let ack = kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(
        !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
        "{:?}",
        ack.reply
    );
    assert!(transition().is_none());
    assert!(panels.take_errors().is_empty());
}

#[test]
fn add_effect_uses_the_seeded_definition_and_renders_with_undo() {
    let actor = setup(json!([]));
    let kernel = actor.handle();
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let plain = pixel(&kernel, &mut renderer);
    assert_eq!(plain, [1., 0., 0., 1.]);
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let button = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("inspector:add-effect"))
        .unwrap();
    finish(
        panels
            .event(
                "inspector",
                publication.generation,
                publication.surface.revision(),
                button.id,
                "click",
                EventDetail::default(),
            )
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let request = loop {
        if let Some(request) = panels
            .take_platform_requests()
            .into_iter()
            .find(|r| r.operation == "effectMenu")
        {
            break request;
        }
        assert!(Instant::now() < deadline, "native menu was not enqueued");
        thread::sleep(Duration::from_millis(1));
    };
    panels.platform_reply(request.id, Ok(json!("invert")));
    while effects(&kernel).is_empty() {
        assert!(
            Instant::now() < deadline,
            "chosen effect did not reach the kernel: {:?}",
            panels.take_errors()
        );
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
        thread::sleep(Duration::from_millis(1));
    }
    let added = &effects(&kernel)[0];
    assert_eq!(added["type"], "invert");
    let current = pm_fx_defs::catalog::main_catalog()
        .current("invert")
        .unwrap();
    assert_eq!(added["version"], current.metadata["version"]);
    let inverted = pixel(&kernel, &mut renderer);
    assert_ne!(inverted, plain, "Invert must change the rendered picture");
    assert_eq!(inverted[3], 1.);
    let ack = kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    assert_eq!(pixel(&kernel, &mut renderer), plain);
    assert!(panels.take_errors().is_empty());
}

#[test]
fn saved_effect_resolves_in_effect_controls_and_renders() {
    let actor = setup(
        json!([{"id":"f1","type":"invert","on":true,"open":true,"version":2,
        "p":{"channel":{"v":0,"kf":[]},"blendWithOriginal":{"v":0,"kf":[]}}}]),
    );
    let kernel = actor.handle();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let shown = texts(&publication.surface);
    assert!(shown.iter().any(|t| t == "Invert"), "{shown:?}");
    assert!(
        !shown.iter().any(|t| t.contains("Missing")),
        "saved effect must resolve: {shown:?}"
    );
    let mut renderer = pm_render::Renderer::headless().unwrap();
    assert_ne!(pixel(&kernel, &mut renderer), [1., 0., 0., 1.]);
    assert!(panels.take_errors().is_empty());
}

#[test]
fn effects_panel_add_button_applies_to_the_selected_layer() {
    let actor = setup(json!([]));
    let kernel = actor.handle();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("fxbrowser").unwrap());
    let row = |panels: &Builtins, text: &str| {
        let publication = panels.take_publications().pop().unwrap();
        let id = publication
            .surface
            .nodes()
            .values()
            .find(|n| n.props.get("label").and_then(Value::as_str) == Some(text))
            .unwrap_or_else(|| panic!("no {text} row"))
            .id;
        (publication, id)
    };
    let (publication, blur) = row(&panels, "Gaussian Blur, effect, Blur & Sharpen");
    finish(
        panels
            .event(
                "fxbrowser",
                publication.generation,
                publication.surface.revision(),
                blur,
                "click",
                EventDetail::default(),
            )
            .unwrap(),
    );
    let (publication, add) = row(&panels, "Add Gaussian Blur");
    finish(
        panels
            .event(
                "fxbrowser",
                publication.generation,
                publication.surface.revision(),
                add,
                "click",
                EventDetail::default(),
            )
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while effects(&kernel).is_empty() {
        assert!(Instant::now() < deadline, "{:?}", panels.take_errors());
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(effects(&kernel)[0]["type"], "blur");
    assert!(panels.take_errors().is_empty());
}

/// A definition that cannot convert keeps failing: one error, then a silent
/// panel without the catalog, would hide it.
#[test]
fn a_malformed_registered_definition_fails_every_sync() {
    let actor = setup(json!([]));
    let kernel = actor.handle();
    kernel
        .state_transaction(pm_kernel::state::StateTransaction {
            expected_revision: None,
            ops: vec![pm_kernel::state::StateOp::Register {
                registry: "effects".into(),
                entry: pm_kernel::state::Registration {
                    token: "synthetic-malformed".into(),
                    owner: Caller::native().writer,
                    id: "synthetic.malformed".into(),
                    definition: data::model(&json!({"frag":"void main(){}","params":"bad"}))
                        .unwrap(),
                },
            }],
        })
        .wait()
        .unwrap()
        .unwrap();
    let mut backend = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "synthetic-registry".into(),
            generation: 1,
            project: "synthetic".into(),
        },
        "layer-effects",
        false,
    )
    .unwrap();
    assert!(backend.sync().is_err());
    assert!(backend.sync().is_err());
}

#[test]
fn transition_properties_duration_uses_a_real_selection_fenced_lease() {
    let actor = setup(json!([]));
    let kernel = actor.handle();
    let spec = pm_kernel::commands::transition_edges::builtin_spec("wipe").unwrap();
    let ack = kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Add Transition".into()),
                    expected_revision: None,
                    body: WriteBody::Commands(vec![
                        pm_kernel::EffectCommand::SetTransition {
                            layer_id: "a".into(),
                            edge: pm_kernel::commands::effects::Edge::In,
                            transition: Some(Box::new(spec)),
                        }
                        .into(),
                        pm_kernel::PropertyCommand::Set(
                            pm_kernel::commands::properties::SetProperty {
                                mode: pm_kernel::commands::properties::KeyMode::Keyframe,
                                ..pm_kernel::commands::properties::SetProperty::new(
                                    "a",
                                    "transitionIn.p.angle",
                                    0.,
                                    0.,
                                )
                            },
                        )
                        .into(),
                    ]),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let mut inspector = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let mut properties = crate::properties::Properties::default();
    let snapshot = properties.snapshot(&mut inspector, 0., false).unwrap();
    let rows = snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap());
    let duration = rows
        .into_iter()
        .find(|r| r["id"] == "transition.in.duration")
        .expect("transition duration row");
    let handle = duration["control"]["axes"][0]["binding"]["handle"]
        .as_str()
        .unwrap();
    let request = properties
        .write(
            &mut inspector,
            Caller::native(),
            handle,
            pm_model::Value::Number(0.75),
            0.,
        )
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let publication = kernel.published("synthetic").unwrap();
    let root = publication.snapshot.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "a").unwrap();
    let layer = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
    assert_eq!(
        layer
            .get("transitionIn")
            .unwrap()
            .get("dur")
            .and_then(pm_model::Value::as_f64),
        Some(0.75)
    );
    assert_eq!(
        publication.history.undo_label.as_deref(),
        Some("Edit Transition")
    );
    inspector.sync(publication);
    let snapshot = properties.snapshot(&mut inspector, 0.25, false).unwrap();
    let angle = snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap())
        .find(|r| r["id"] == "transition.in.p.angle")
        .unwrap();
    let handle = angle["control"]["axes"][0]["binding"]["handle"]
        .as_str()
        .unwrap();
    let request = properties
        .write(
            &mut inspector,
            Caller::native(),
            handle,
            pm_model::Value::Number(37.),
            0.25,
        )
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let publication = kernel.published("synthetic").unwrap();
    let root = publication.snapshot.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "a").unwrap();
    let layer = data::wire(&pm_kernel::model::layer_at(root, &loc).unwrap().to_value());
    let keys = layer["transitionIn"]["p"]["angle"]["kf"]
        .as_array()
        .unwrap();
    assert_eq!(
        keys.len(),
        2,
        "editing an animated parameter adds a key instead of changing only its hidden base value"
    );
    assert!(keys.iter().any(|key| key["v"] == 37.));
    let ack = kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Change Transition".into()),
                    expected_revision: None,
                    body: WriteBody::Commands(vec![
                        pm_kernel::EffectCommand::SetTransition {
                            layer_id: "a".into(),
                            edge: pm_kernel::commands::effects::Edge::In,
                            transition: pm_kernel::commands::transition_edges::builtin_spec(
                                "crossfade",
                            )
                            .map(Box::new),
                        }
                        .into(),
                    ]),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(
        !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
        "{:?}",
        ack.reply
    );
    inspector.sync(kernel.published("synthetic").unwrap());
    assert!(
        properties
            .write(
                &mut inspector,
                Caller::native(),
                handle,
                pm_model::Value::Number(90.),
                0.25
            )
            .is_err(),
        "a stale Wipe angle control must not create a channel on Crossfade"
    );
}

#[test]
fn v4_effect_registration_enters_the_kernel_and_disposal_removes_it() {
    let actor = setup(json!([{ "id":"f1", "type":"v4-probe.fx", "on":true,"p":{} }]));
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "export async function mount(env) { env.effects.register({id:'v4-probe.fx',label:'Probe',group:'Test',params:[{k:'amount',label:'Amount',min:0,max:1,def:0.5}],frag:'o=src(v_uv)*u_amount;',passes:2,keepOrig:true}); const root=env.root(); await root.render(null); return {root,dispose:()=>root.dispose()}; }"
        },
        "edges": { "powermove:bundle": {"./index.tsx":"index.tsx"}, "index.tsx": {} }
    });
    finish(panels.mount_extension("v4-probe", &graph, None).unwrap());
    let state = actor.handle().state();
    let provider = state.provider("effects", "v4-probe.fx").unwrap();
    let definition = data::wire(&provider.definition);
    assert_eq!(definition["params"][0]["def"], 0.5);
    let snapshot: pm_protocol::definitions::DefinitionSnapshot =
        serde_json::from_value(definition["nativeSnapshot"].clone()).unwrap();
    assert_eq!(snapshot.flags.passes, 2);
    assert!(snapshot.flags.keep_orig);
    assert!(
        snapshot
            .glsl
            .as_ref()
            .unwrap()
            .source
            .contains("uniform float u_amount;")
    );
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let effected = pixel_with_extensions(&actor.handle(), &mut renderer, &[snapshot]);
    assert!(effected[0] < 0.6 && effected[3] < 0.6, "{effected:?}");
    panels.unmount_extension("v4-probe");
    assert!(
        actor
            .handle()
            .state()
            .provider("effects", "v4-probe.fx")
            .is_none()
    );
    renderer.set_bypass_missing_effects(true);
    assert_eq!(pixel(&actor.handle(), &mut renderer), [1., 0., 0., 1.]);
    assert_eq!(effects(&actor.handle())[0]["type"], "v4-probe.fx");
    panels.close();
    actor.shutdown();
}

#[test]
fn unavailable_extension_keeps_channels_and_draws_the_layer() {
    let saved = json!([{"id":"saved-pin","type":"removed.fx","on":true,"open":true,
        "p":{"amount":{"v":0.75,"kf":[{"t":1,"v":0.25,"i":"key"}]}}}]);
    let actor = setup(saved.clone());
    let kernel = actor.handle();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    let shown = texts(&panels.take_publications().pop().unwrap().surface);
    assert!(
        shown.iter().any(|text| text == "Not installed"),
        "{shown:?}"
    );
    let mut renderer = pm_render::Renderer::headless().unwrap();
    renderer.set_bypass_missing_effects(true);
    assert_eq!(pixel(&kernel, &mut renderer), [1., 0., 0., 1.]);
    assert_eq!(effects(&kernel), saved.as_array().unwrap().clone());
    assert!(panels.take_errors().is_empty());
    panels.close();
    actor.shutdown();
}

#[test]
fn v4_provider_order_survives_unrelated_registration_and_override_disposal() {
    let actor = setup(json!([]));
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = |label: &str| {
        json!({"sources":{
        "powermove:bundle":"export * from './index.tsx';",
        "index.tsx":format!("export async function mount(env){{globalThis.__effects=env.effects;env.effects.register({{id:'shared.fx',label:'{label}',group:'Test',params:[],frag:'o=src(v_uv);'}});const root=env.root();await root.render(null);return {{root,dispose:()=>root.dispose()}};}}")
    },"edges":{"powermove:bundle":{"./index.tsx":"index.tsx"},"index.tsx":{}}})
    };
    finish(
        panels
            .mount_extension("provider-a", &graph("A"), None)
            .unwrap(),
    );
    finish(
        panels
            .mount_extension("provider-b", &graph("B"), None)
            .unwrap(),
    );
    let label = || {
        let state = actor.handle().state();
        state.provider("effects", "shared.fx").map(|entry| {
            entry
                .definition
                .get("label")
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
    };
    assert_eq!(label().as_deref(), Some("B"));
    finish(panels.realm_of("provider-a").unwrap().eval(
        "__effects.register({id:'unrelated.fx',label:'Other',group:'Test',params:[],frag:'o=src(v_uv);'});true"
    ).unwrap());
    assert_eq!(
        label().as_deref(),
        Some("B"),
        "adding another effect must not reclaim a shadowed id"
    );
    finish(panels.realm_of("provider-a").unwrap().eval(
        "globalThis.__top=__effects.register({id:'shared.fx',label:'A2',group:'Test',params:[],frag:'o=src(v_uv);'});true"
    ).unwrap());
    assert_eq!(label().as_deref(), Some("A2"));
    finish(
        panels
            .realm_of("provider-a")
            .unwrap()
            .eval("__top();true")
            .unwrap(),
    );
    assert_eq!(
        label().as_deref(),
        Some("B"),
        "dispose reveals the original global stack order"
    );
    panels.unmount_extension("provider-b");
    assert_eq!(label().as_deref(), Some("A"));
    panels.unmount_extension("provider-a");
    assert!(label().is_none());
    panels.close();
    actor.shutdown();
}
