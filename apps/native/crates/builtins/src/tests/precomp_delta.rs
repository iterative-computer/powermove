//! E12 exercises the authored v4 controls and real published export path.
use super::*;

fn setup() -> KernelActor {
    let actor = KernelActor::spawn(KernelConfig::default());
    let p=data::model(&json!({"id":"synthetic","name":"Precomp contract","compId":"root","w":64,"h":64,"fps":30,"dur":3,"colorDepth":16,"bg":"#000000","backgroundFill":{"type":"none","angle":0,"stops":[]},"assets":{},"layers":[{"id":"pre","type":"precomp","name":"Nested picture","from":0,"dur":3,"on":true,"parent":null,"group":null,"p":{"position.x":{"v":0,"kf":[]},"position.y":{"v":0,"kf":[]},"anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]},"scale.x":{"v":400,"kf":[]},"scale.y":{"v":400,"kf":[]},"rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{"comp":"sub","w":16,"h":16,"trim":0,"speed":1},"fx":[]}],"comps":{"sub":{"id":"sub","name":"Source","w":16,"h":16,"fps":30,"dur":3,"bg":"#000000","backgroundFill":{"type":"none","angle":0,"stops":[]},"layers":[{"id":"stripe","type":"solid","name":"Stripe","from":0,"dur":3,"on":true,"p":{"position.x":{"v":4,"kf":[]},"position.y":{"v":0,"kf":[]},"anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]},"scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},"rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}},"d":{"w":0.5,"h":16,"color":"#FFFFFF"}}]}}})).unwrap();
    actor
        .handle()
        .open("synthetic", OpenSource::Untitled(p))
        .wait()
        .unwrap()
        .unwrap();
    select_layer(&actor.handle(), "pre");
    actor
}
fn render(
    kernel: &pm_kernel::KernelHandle,
    renderer: &mut pm_render::Renderer,
    time: f64,
) -> Vec<f32> {
    let p = kernel.published("synthetic").unwrap();
    let mut request = pm_render::RenderRequest::sized(
        pm_protocol::common::Size::new(64, 64),
        pm_protocol::color::Precision::Float16,
    );
    request.export = true;
    request.output_kind = pm_render::OutputKind::Final;
    let frame = pm_export::render::render_frame(
        renderer,
        &p.snapshot.document,
        "root",
        pm_protocol::common::Ticks::new((time * 705_600_000.).round() as i64).unwrap(),
        &request,
        &Default::default(),
        &pm_export::render::Unrestricted,
        1,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    frame.rgba
}
fn layer(kernel: &pm_kernel::KernelHandle) -> Value {
    let p = kernel.published("synthetic").unwrap();
    let root = p.snapshot.root().unwrap();
    data::wire(
        &pm_kernel::model::layer_at(root, &pm_kernel::model::find_layer(root, "pre").unwrap())
            .unwrap()
            .to_value(),
    )
}
fn history(kernel: &pm_kernel::KernelHandle, action: pm_kernel::HistoryCall) {
    let ack = kernel
        .call("synthetic", Request::new(Caller::native(), action))
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
}
fn publish(panels: &mut Builtins) {
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }
}
#[test]
fn e12_application_continuous_raster_switch_reaches_saved_layer_and_sharp_export_with_undo() {
    let actor = setup();
    let kernel = actor.handle();
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let original = render(&kernel, &mut renderer, 0.);
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let toggle = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("value:layer.continuousRaster"))
        .expect("actual Continuously Rasterize switch");
    assert_eq!(toggle.props.get("value"), Some(&json!(false)));
    finish(
        panels
            .write(
                "inspector",
                toggle.props["binding"].as_str().unwrap(),
                json!(true),
            )
            .unwrap(),
    );
    assert_eq!(layer(&kernel)["continuousRaster"], true);
    let sharp = render(&kernel, &mut renderer, 0.);
    assert_ne!(
        sharp, original,
        "display density must change the subpixel source raster"
    );
    for y in 0..64 {
        for x in 0..64 {
            let p = &sharp[(y * 64 + x) * 4..][..4];
            let expected = if (16..18).contains(&x) { 1. } else { 0. };
            assert_eq!(p, [expected; 4], "display-density stripe at {x},{y}");
        }
    }
    history(&kernel, pm_kernel::HistoryCall::Undo);
    assert_eq!(render(&kernel, &mut renderer, 0.), original);
    history(&kernel, pm_kernel::HistoryCall::Redo);
    assert_eq!(render(&kernel, &mut renderer, 0.), sharp);
    assert!(panels.take_errors().is_empty());
}
#[test]
fn e12_application_release_precomp_action_preserves_published_picture_and_undo_redo() {
    let actor = setup();
    let kernel = actor.handle();
    let mut model = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let request = model
        .set(
            Caller::native(),
            pm_panel_inspector::Binding::Layer {
                layer: "pre".into(),
                field: "continuousRaster".into(),
            },
            pm_model::Value::Bool(true),
            0.,
        )
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let original = render(&kernel, &mut renderer, 0.);
    let before = kernel
        .published("synthetic")
        .unwrap()
        .snapshot
        .document
        .to_value();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let button = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("label").and_then(Value::as_str) == Some("Release Pre-comp"))
        .expect("actual release action");
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
    while layer(&kernel)["type"] != "group" {
        assert!(
            Instant::now() < deadline,
            "release acknowledgement: {:?}",
            panels.take_errors()
        );
        publish(&mut panels);
        thread::sleep(Duration::from_millis(1));
    }
    for time in [0., 1., 2.9, 0.5] {
        assert_eq!(
            render(&kernel, &mut renderer, time),
            original,
            "released image at {time}"
        );
    }
    history(&kernel, pm_kernel::HistoryCall::Undo);
    assert_eq!(
        kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .document
            .to_value(),
        before
    );
    assert_eq!(render(&kernel, &mut renderer, 0.), original);
    history(&kernel, pm_kernel::HistoryCall::Redo);
    assert_eq!(layer(&kernel)["type"], "group");
    assert_eq!(render(&kernel, &mut renderer, 0.), original);
    assert!(panels.take_errors().is_empty());
}
