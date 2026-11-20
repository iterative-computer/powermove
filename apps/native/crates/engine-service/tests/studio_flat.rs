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
        track: None,
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
    cube_between_2d_layers(false);
}

/// The cube added to an empty composition first, then 2D layers above and
/// below it: no studio floor or sky hides the layer below.
#[test]
fn a_cube_added_before_the_2d_layers_keeps_the_layers_below_and_above_it_visible() {
    cube_between_2d_layers(true);
}

fn cube_between_2d_layers(cube_first: bool) {
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
    if !cube_first {
        transact(&actor, vec![solid("below")]);
    }
    let published = actor.handle().published("p").unwrap();
    let (_, commands, _) =
        pm_panel_studio3d::properties::add_layer(&published, 0., "box", "cube").unwrap();
    transact(&actor, commands);
    if cube_first {
        transact(&actor, vec![solid("below")]);
    }
    transact(&actor, vec![solid("above")]);
    let published = actor.handle().published("p").unwrap();
    let mut root = from_model(&published.snapshot.root().unwrap().to_value()).unwrap();
    if cube_first {
        // The layer below goes to the bottom of the stack.
        let all = layers(&mut root);
        let at = all.iter().position(|l| l["id"] == "below").unwrap();
        let below = all.remove(at);
        all.push(below);
    }
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

/// Where a point of the scene lands in a `size`-pixel square frame seen by
/// the default camera (no camera layer): at (3, 2, 6), aimed at the origin,
/// 45° vertical field of view.
fn pixel(p: [f64; 3], size: u32) -> (u32, u32) {
    let eye = [3., 2., 6.];
    let norm = |a: [f64; 3]| {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        a.map(|x| x / l)
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let f = norm([-eye[0], -eye[1], -eye[2]]);
    let r = norm(cross(f, [0., 1., 0.]));
    let u = cross(r, f);
    let t = 22.5f64.to_radians().tan();
    let d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
    let (x, y) = (dot(d, r) / dot(d, f) / t, dot(d, u) / dot(d, f) / t);
    let s = size as f64;
    (((x + 1.) / 2. * s) as u32, ((1. - y) / 2. * s) as u32)
}

/// The cube Layer ▸ Add 3D Layer adds to an unlit composition: its face
/// turned away from the key light still reads (the scene's default ambient
/// fills it), and stays darker than the face the key lights.
#[test]
fn the_menu_cube_shadow_side_reads_against_its_lit_side() {
    const FRAME: u32 = 128;
    let actor = KernelActor::spawn(KernelConfig::default());
    actor
        .handle()
        .open(
            "p",
            OpenSource::Untitled(to_model(&json!({"id":"p","compId":"c","w":FRAME,"h":FRAME,
                "fps":30,"dur":1,"layers":[],"assets":{},"comps":{}}))),
        )
        .wait()
        .unwrap()
        .unwrap();
    let published = actor.handle().published("p").unwrap();
    let (_, commands, _) =
        pm_panel_studio3d::properties::add_layer(&published, 0., "box", "cube").unwrap();
    transact(&actor, commands);
    let published = actor.handle().published("p").unwrap();
    let mut root = from_model(&published.snapshot.root().unwrap().to_value()).unwrap();
    let cube = layers(&mut root)
        .iter()
        .find(|l| l["id"] == "cube")
        .unwrap()
        .clone();
    let side = cube["d"]["data"]["object"]["source"]["parameters"]["width"]
        .as_f64()
        .unwrap_or(1.)
        * cube["p"]["scale.x"]["v"].as_f64().unwrap_or(100.)
        / 100.;
    let quality = pm_scene3d::quality::resolve(
        &pm_scene3d::quality::preset("production").unwrap(),
        &json!({"minSamples":16,"maxSamples":16,"noiseThreshold":0,"caustics":false}),
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
    let h = side / 2.;
    // The key is up and to the left, in front: it lights the front face.
    // The right face is turned away from it. Both in the export and in the
    // viewer.
    let mut ratios = vec![];
    for export in [true, false] {
        let mut renderer = Renderer::headless().unwrap();
        studio::attach(&mut renderer, doc.clone(), &Default::default());
        let mut request = RenderRequest::sized(Size::new(FRAME, FRAME), Precision::Float16);
        request.export = export;
        request.output_kind = if export {
            OutputKind::Final
        } else {
            OutputKind::Preview
        };
        request.background = false;
        let frame = renderer
            .render(&doc, "c", Ticks::ZERO, &request, &pm_render::MapResolver::default())
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let pixels = renderer.read_rgba(&frame).unwrap();
        // Mean luminance over a 5×5 patch around a face's centre.
        let face = |centre: [f64; 3]| {
            let (cx, cy) = pixel(centre, FRAME);
            let mut sum = 0.;
            for y in cy - 2..=cy + 2 {
                for x in cx - 2..=cx + 2 {
                    let i = ((y * FRAME + x) * 4) as usize;
                    let p = &pixels[i..i + 4];
                    assert!(p[3] > 0.99, "the cube covers {x},{y}: {p:?}");
                    sum += 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
                }
            }
            sum / 25.
        };
        let lit = face([0., 0., h]);
        let dark = face([h, 0., 0.]);
        let ratio = dark / lit;
        ratios.push((export, lit, dark, ratio));
    }
    for (export, lit, dark, ratio) in ratios {
        assert!(
            (0.30..=0.95).contains(&ratio),
            "export {export}: the shadow side is {:.0}% of the lit side ({dark:.4} / {lit:.4})",
            ratio * 100.
        );
    }
}
