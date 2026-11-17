//! A 3D layer among 2D layers composites in stack order over a clear
//! background: a cube added with Layer ▸ Add 3D Layer into a composition that
//! has 2D layers leaves the layer below it and the layer above it visible (no
//! studio floor, no visible sky).
#![cfg(feature = "kernel")]
use pm_engine_service::studio;
use pm_kernel::{
    Call, Caller, Command, KernelActor, KernelConfig, LayerCommand, OpenSource, Reply, Request,
    WriteBody,
};
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{OutputKind, RenderRequest, Renderer};
use pm_scene3d::{from_model, to_model};
use serde_json::{Value, json};

const SIZE: u32 = 64;

fn transact(actor: &KernelActor, commands: Vec<Command>) {
    let ack = actor
        .handle()
        .call(
            "p",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: None,
                    body: WriteBody::Commands(commands),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
}
fn solid(id: &str) -> Command {
    LayerCommand::Add(Box::new(pm_kernel::commands::layers::AddLayer {
        id: id.into(),
        layer_type: "solid".into(),
        comp: None,
        name: None,
        from: None,
        duration: None,
        content: Default::default(),
        properties: Default::default(),
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time: 0.,
    }))
    .into()
}
/// The composition's layers, wherever the root keeps them.
fn layers(root: &mut Value) -> &mut Vec<Value> {
    if root["comps"]["c"]["layers"].is_array() {
        root["comps"]["c"]["layers"].as_array_mut().unwrap()
    } else {
        root["layers"].as_array_mut().unwrap()
    }
}

#[test]
fn a_cube_added_among_2d_layers_keeps_the_layers_below_and_above_it_visible() {
    let actor = KernelActor::spawn(KernelConfig::default());
    actor
        .handle()
        .open(
            "p",
            OpenSource::Untitled(to_model(&json!({"id":"p","compId":"c","w":SIZE,"h":SIZE,
                "fps":30,"dur":1,"layers":[],"assets":{},"comps":{}}))),
        )
        .wait()
        .unwrap()
        .unwrap();
    // A 2D layer, then a cube as the menu adds it, then a 2D layer over it.
    transact(&actor, vec![solid("below")]);
    let published = actor.handle().published("p").unwrap();
    let (_, commands, _) =
        pm_panel_studio3d::properties::add_layer(&published, 0., "box", "cube").unwrap();
    transact(&actor, commands);
    transact(&actor, vec![solid("above")]);
    let published = actor.handle().published("p").unwrap();
    let mut root = from_model(&published.snapshot.root().unwrap().to_value()).unwrap();
    let names: Vec<String> = layers(&mut root)
        .iter()
        .map(|l| l["name"].as_str().unwrap_or_default().to_owned())
        .collect();
    for layer in layers(&mut root).iter_mut() {
        match layer["id"].as_str() {
            // Red over the whole frame, under the cube.
            Some("below") => {
                layer["d"]["color"] = json!("#FF0000");
                layer["d"]["w"] = json!(SIZE);
                layer["d"]["h"] = json!(SIZE);
            }
            // A green square in the top-left corner, over the cube.
            Some("above") => {
                layer["d"]["color"] = json!("#00FF00");
                layer["d"]["w"] = json!(16);
                layer["d"]["h"] = json!(16);
                layer["p"]["position.x"]["v"] = json!(8);
                layer["p"]["position.y"]["v"] = json!(8);
            }
            _ => {}
        }
    }
    // Few samples: this is about what shows, not how finely it's traced.
    let quality = pm_scene3d::quality::resolve(
        &pm_scene3d::quality::preset("production").unwrap(),
        &json!({"minSamples":4,"maxSamples":4,"noiseThreshold":0,"caustics":false}),
    )
    .unwrap();
    if root["comps"]["c"].is_object() {
        root["comps"]["c"]["studioQuality"] = quality;
    } else {
        root["studioQuality"] = quality;
    }
    root["colorDepth"] = json!(16);
    let doc = pm_kernel::DocumentStore::new_untitled(to_model(&root))
        .snapshot()
        .document;
    let mut renderer = Renderer::headless().unwrap();
    studio::attach(&mut renderer, doc.clone(), &Default::default());
    let mut request = RenderRequest::sized(Size::new(SIZE, SIZE), Precision::Float16);
    request.export = true;
    request.output_kind = OutputKind::Final;
    request.background = false;
    let frame = renderer
        .render(
            &doc,
            "c",
            Ticks::ZERO,
            &request,
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * SIZE + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    let red = |p: [f32; 4]| p[0] > 0.9 && p[1] < 0.1 && p[2] < 0.1 && p[3] > 0.99;
    let green = |p: [f32; 4]| p[1] > 0.9 && p[0] < 0.1 && p[2] < 0.1;
    if std::env::var_os("PM_STUDIO_FLAT_DUMP").is_some() {
        for y in (0..SIZE).step_by(4) {
            let row: String = (0..SIZE)
                .step_by(2)
                .map(|x| match at(x, y) {
                    p if red(p) => 'r',
                    p if green(p) => 'g',
                    p if p[3] < 0.01 => ' ',
                    _ => '#',
                })
                .collect();
            eprintln!("{row}");
        }
    }
    // The layer above the cube shows in its corner.
    assert!(green(at(14, 14)), "the 2D layer above: {:?}", at(14, 14));
    // The layer below shows wherever the cube isn't: the frame's edges.
    for (x, y) in [(SIZE - 3, SIZE - 3), (SIZE - 3, 3), (3, SIZE - 3)] {
        assert!(
            red(at(x, y)),
            "the 2D layer below at {x},{y}: {:?}",
            at(x, y)
        );
    }
    // And the cube draws over it in the middle.
    let covered = (0..SIZE * SIZE)
        .filter(|i| !red(at(i % SIZE, i / SIZE)) && !green(at(i % SIZE, i / SIZE)))
        .count();
    assert!(covered > 40, "the cube covers {covered} pixels");
    assert!(
        !names.iter().any(|n| n == "Floor"),
        "no studio floor among 2D layers: {names:?}"
    );
}
